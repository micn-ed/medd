//! Coordinates a clean shutdown (plan-v0.1.md's fifth blocker, alongside the Cmd+W menu fix):
//! quitting medd must flush every pending autosave before the process actually exits, or any
//! edit still inside its ~1s debounce window — in any open tab — is lost. Increment 7's
//! close-flush work (`doc/doc.ts`'s `pending` map and its `waitForAllQuiescent`/`flushAll`) is
//! reused unchanged here: this module's only job is the Rust-side half of asking for that flush
//! and waiting for it, bounded, before letting the app actually die.
//!
//! The invariant, same shape as the close-flush's: the exit must issue everything the debounce
//! still owes, and the outcome of those writes applies to nothing — nothing here needs to know or
//! care whether a flushed write actually landed, only that it was *issued* through the same
//! `document_write` path every other write goes through. There is no shortcut write for shutdown;
//! a quit handler that wrote directly, bypassing the compare-and-swap, would satisfy every other
//! requirement here while quietly giving up the one atomic-write guarantee (increment 2) that
//! makes a process dying mid-write leave the original file untouched.
//!
//! A conflicted or detached tab is not flushed — `doc.ts`'s `requestWrite` already suspends
//! autosave for those, and quitting must not silently resolve a conflict in either direction on
//! the user's behalf. The banner already on screen (or lack of one, for a detached tab) is the
//! only warning that edit does not survive the quit.
//!
//! Two Tauri events reach this coordinator, from `main.rs`: `RunEvent::ExitRequested` (Cmd+Q's
//! custom menu item, or `AppHandle::exit()` called programmatically) and
//! `RunEvent::WindowEvent { event: WindowEvent::CloseRequested, .. }` (the traffic light,
//! Cmd+Shift+W). They must share this one coordinator rather than each growing its own flush
//! logic: an architect review of the first version found that `CloseRequested` reaches
//! `ExitRequested` only *after* the window — and with it the webview the flush needs to run in —
//! is already destroyed, so a version that only handled `ExitRequested` silently flushed nothing
//! on every window-close route.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// How long the app waits for the frontend to report every pending write has landed before
/// exiting anyway. This is a budget, not a guess: `flushAll` (`doc/doc.ts`) cancels every pending
/// debounce timer and issues the writes immediately, so the ~1s autosave debounce is *not* part of
/// what this waits out. What it actually covers is N serialised `DocumentStore::write` calls (one
/// `fsync` each, and Rust's own store-wide mutex means they queue rather than overlap) — at a
/// pessimistic ten dirty tabs, with each `fsync` running tens to low hundreds of milliseconds under
/// concurrent filesystem load (a `git checkout`, Spotlight indexing), that's on the order of one
/// second. 3s is roughly 3x that worst case. It is not scaled by the number of dirty tabs on
/// purpose: the ceiling exists to bound a write that *never settles*, and a stalled write doesn't
/// get more dangerous as more tabs are open, so an adaptive bound would perversely grant the
/// pathological case more time exactly when there's more to lose. This number belongs on
/// increment 12's measurement list, same category as the large-document thresholds, once there's a
/// soak test to check it against rather than the budget above.
pub const QUIT_FLUSH_CEILING: Duration = Duration::from_secs(3);

/// What an event handler in `main.rs` should do about one exit-shaped event (`ExitRequested`, or
/// `WindowEvent::CloseRequested`). Returned by `QuitCoordinator::decide` so that *deciding* stays
/// out of `main.rs` entirely — architecture.md §2: "a shell contains no decisions; if a
/// Tauri-aware function has a branch in it, it is in the wrong place." The first version of this
/// module put the decision in `main.rs`'s own `RunEvent` match, which is exactly the shape that
/// left it with no test seam: the same tell as `watcher.rs`'s `run_event_loop`, where the function
/// holding the `AppHandle` turned out to be the one nothing could reach. `main.rs` now only ever
/// asks `decide()` a question and acts on the two fields below — no branch of its own beyond
/// calling `prevent_exit`/`prevent_close` or not, and starting the flush thread or not.
pub struct ExitDecision {
    /// Whether the caller must call `prevent_exit()`/`prevent_close()` on this event.
    pub prevent: bool,
    /// `Some(rx)` exactly when this is the request that should start the flush-and-wait thread —
    /// the first request, and only the first. `None` covers both "already flushing" (a repeat
    /// request, still `prevent`ed) and "ready to exit" (not `prevent`ed at all).
    pub start_flush: Option<mpsc::Receiver<()>>,
}

/// Coordinates the one shutdown flush, however it was triggered. Two pieces of state, deliberately
/// not one flag doing double duty (an architect review found the original single-flag version
/// conflated two different callers that need opposite treatment):
///
/// - `shutting_down` — set the instant `decide()` first runs. Its job is purely "has a flush
///   already been started", so a second, overlapping trigger (a stray `CloseRequested` while an
///   `ExitRequested`-driven flush is already running, or vice versa) doesn't spawn a second flush
///   thread racing the first.
/// - `ready_to_exit` — set only by this module's own background thread, once the frontend has
///   reported done or the ceiling has expired. This is the *only* condition under which `decide()`
///   answers `prevent: false`. A repeat user request (pressing Cmd+Q again, or clicking the close
///   button again) before this is set is *not* treated as "yes, I mean it" — an architect review
///   reversed that original design: because a healthy flush is a handful of writes finishing in
///   milliseconds, medd will almost always have exited before a human can press twice, so a
///   "second press skips the wait" rule does nothing in the common case and, in the rare case it
///   *does* fire, does it exactly when a write is stalled and the user's edit is most at risk. A
///   repeat press now just prevents again and changes nothing; the ceiling remains the only way
///   out of a genuinely stuck flush, and force-quit remains for a genuinely stuck ceiling.
///
/// One more dependency worth stating because nothing enforces it: **this is safe as a one-way
/// latch only because nothing here can cancel a quit once it has begun.** If a future "you have
/// unresolved conflicts — really quit?" prompt ever makes the decision reversible, both this
/// coordinator's latch and `doc.ts`'s `isShuttingDown` latch need an explicit clear path added on
/// cancel, or autosave stays silently off for the rest of the session — the worst-shaped bug this
/// product can have.
pub struct QuitCoordinator {
    shutting_down: Mutex<bool>,
    ready_to_exit: Mutex<bool>,
    tx: Mutex<Option<mpsc::Sender<()>>>,
}

impl QuitCoordinator {
    pub fn new() -> Self {
        QuitCoordinator {
            shutting_down: Mutex::new(false),
            ready_to_exit: Mutex::new(false),
            tx: Mutex::new(None),
        }
    }

    /// The single decision point for both exit-shaped events. See `ExitDecision` for why this
    /// exists and what a caller does with the result.
    pub fn decide(&self) -> ExitDecision {
        if self.is_ready_to_exit() {
            return ExitDecision {
                prevent: false,
                start_flush: None,
            };
        }
        ExitDecision {
            prevent: true,
            start_flush: self.begin_shutdown(),
        }
    }

    /// `Some(rx)` the first call only — the flush-and-wait thread should be started on it. `None`
    /// on every call after, meaning a flush is already underway and there is nothing further to
    /// start.
    fn begin_shutdown(&self) -> Option<mpsc::Receiver<()>> {
        let mut shutting_down = self.shutting_down.lock().unwrap();
        if *shutting_down {
            return None;
        }
        *shutting_down = true;

        let (tx, rx) = mpsc::channel();
        *self.tx.lock().unwrap() = Some(tx);
        Some(rx)
    }

    /// Whether the flush-and-wait thread has finished (by signal or by ceiling) and is about to
    /// call `AppHandle::exit()` itself.
    fn is_ready_to_exit(&self) -> bool {
        *self.ready_to_exit.lock().unwrap()
    }

    /// Called by the flush-and-wait thread immediately before it triggers the real exit, so the
    /// `RunEvent` that call produces is recognised (via `decide()`) as the one to let through.
    pub fn mark_ready_to_exit(&self) {
        *self.ready_to_exit.lock().unwrap() = true;
    }

    /// Called from the `quit_ready` command once the frontend has flushed and awaited quiescence.
    /// Takes the sender so a second, stray call is a harmless no-op rather than a second signal
    /// on a channel whose receiver may already be gone.
    pub fn signal_ready(&self) {
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

impl Default for QuitCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Blocks until either `rx` receives the frontend's ready signal or `ceiling` elapses, returning
/// which happened. A thin wrapper over `recv_timeout` — pulled out on its own so the timing
/// behaviour itself is testable without spinning up a real Tauri app: nothing here about *why*
/// the frontend is slow (or fast) is Tauri-specific.
pub fn wait_for_quit_signal(rx: &mpsc::Receiver<()>, ceiling: Duration) -> bool {
    rx.recv_timeout(ceiling).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn returns_true_promptly_when_the_signal_arrives_before_the_ceiling() {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            tx.send(()).unwrap();
        });

        let start = std::time::Instant::now();
        assert!(wait_for_quit_signal(&rx, Duration::from_secs(5)));
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "should return as soon as the signal arrives, not wait out the full ceiling"
        );
    }

    #[test]
    fn returns_false_once_the_ceiling_elapses_with_no_signal() {
        // NOTE for whoever meets a hung test suite here: a mutant that replaces the bounded
        // `recv_timeout` with an unbounded `recv()` does not fail this test -- it hangs it,
        // because `_tx` below is deliberately kept alive so the channel never disconnects on its
        // own. A suite that blocks instead of going red on this file means look here first, not
        // at the environment.
        let (_tx, rx) = mpsc::channel::<()>();
        // _tx is kept alive (not dropped) so this exercises the timeout path specifically,
        // rather than the "sender dropped" disconnect path recv_timeout also treats as an error.

        let start = std::time::Instant::now();
        assert!(!wait_for_quit_signal(&rx, Duration::from_millis(50)));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    // `decide()` is the seam an architect review asked for: the repeat-press behaviour used to be
    // a branch inside main.rs's Tauri-aware RunEvent handler, which is exactly the shape
    // (architecture.md §2's "a shell contains no decisions") that left it untestable without
    // spinning up a real app. These tests pin it directly, no Tauri types involved.

    #[test]
    fn the_first_decision_prevents_and_starts_the_flush() {
        let coordinator = QuitCoordinator::new();
        let decision = coordinator.decide();
        assert!(decision.prevent);
        assert!(decision.start_flush.is_some());
    }

    #[test]
    fn a_repeat_decision_before_ready_prevents_again_and_starts_nothing() {
        let coordinator = QuitCoordinator::new();
        coordinator.decide(); // the first request, already started a flush

        let repeat = coordinator.decide(); // a second press, or CloseRequested racing it

        assert!(
            repeat.prevent,
            "a repeat request must still be prevented -- 'yes, I mean it' was the reversed ruling"
        );
        assert!(
            repeat.start_flush.is_none(),
            "must not start a second flush thread racing the first"
        );
    }

    #[test]
    fn once_ready_the_decision_stops_preventing_and_starts_nothing_new() {
        let coordinator = QuitCoordinator::new();
        coordinator.decide();
        coordinator.mark_ready_to_exit();

        let decision = coordinator.decide();

        assert!(
            !decision.prevent,
            "the flush thread's own completion exit() call must be allowed through"
        );
        assert!(decision.start_flush.is_none());
    }

    #[test]
    fn signal_ready_wakes_the_receiver_from_the_first_decision() {
        let coordinator = QuitCoordinator::new();
        let rx = coordinator.decide().start_flush.unwrap();

        coordinator.signal_ready();

        assert!(wait_for_quit_signal(&rx, Duration::from_secs(1)));
    }

    #[test]
    fn signal_ready_before_any_shutdown_is_a_harmless_no_op() {
        let coordinator = QuitCoordinator::new();
        coordinator.signal_ready(); // nothing to signal yet -- must not panic
    }
}
