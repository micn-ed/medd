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

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// How long the app waits for the frontend to report every pending write has landed before
/// exiting anyway. Deliberately a few seconds, not longer: an editor that cannot be quit because
/// one write never settles is worse than losing that one write, and the user forcing a kill loses
/// everything else in flight too. Deliberately not shorter: truncating an ordinary flush that was
/// about to finish loses the very edit this exists to save. Chosen to comfortably exceed the
/// ~1s autosave debounce plus a real (not stalled) disk write, with headroom.
pub const QUIT_FLUSH_CEILING: Duration = Duration::from_secs(3);

/// Tracks whether a shutdown flush is already underway, and the channel the frontend's
/// `quit_ready` command signals on once it has flushed everything it can. `AppHandle::exit()`
/// itself re-triggers `RunEvent::ExitRequested` — without this guard, the exit issued once the
/// flush completes (or times out) would loop back into `begin_shutdown` and prevent itself again,
/// forever.
pub struct QuitCoordinator {
    shutting_down: Mutex<bool>,
    tx: Mutex<Option<mpsc::Sender<()>>>,
}

impl QuitCoordinator {
    pub fn new() -> Self {
        QuitCoordinator {
            shutting_down: Mutex::new(false),
            tx: Mutex::new(None),
        }
    }

    /// Called from `RunEvent::ExitRequested`. `Some(rx)` the first time — the caller should
    /// prevent the exit and wait on the returned receiver — `None` on every call after, which is
    /// what the exit this module itself triggers must see so it can actually go through.
    pub fn begin_shutdown(&self) -> Option<mpsc::Receiver<()>> {
        let mut shutting_down = self.shutting_down.lock().unwrap();
        if *shutting_down {
            return None;
        }
        *shutting_down = true;

        let (tx, rx) = mpsc::channel();
        *self.tx.lock().unwrap() = Some(tx);
        Some(rx)
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
        let (_tx, rx) = mpsc::channel::<()>();
        // _tx is kept alive (not dropped) so this exercises the timeout path specifically,
        // rather than the "sender dropped" disconnect path recv_timeout also treats as an error.

        let start = std::time::Instant::now();
        assert!(!wait_for_quit_signal(&rx, Duration::from_millis(50)));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn begin_shutdown_returns_a_receiver_only_the_first_time() {
        let coordinator = QuitCoordinator::new();
        assert!(coordinator.begin_shutdown().is_some());
        assert!(
            coordinator.begin_shutdown().is_none(),
            "a second ExitRequested -- the one this module's own exit() call re-triggers -- must \
             not prevent the exit all over again"
        );
    }

    #[test]
    fn signal_ready_wakes_the_receiver_returned_by_begin_shutdown() {
        let coordinator = QuitCoordinator::new();
        let rx = coordinator.begin_shutdown().unwrap();

        coordinator.signal_ready();

        assert!(wait_for_quit_signal(&rx, Duration::from_secs(1)));
    }

    #[test]
    fn signal_ready_before_any_shutdown_is_a_harmless_no_op() {
        let coordinator = QuitCoordinator::new();
        coordinator.signal_ready(); // nothing to signal yet -- must not panic
    }
}
