//! `route_open`, the pending-open buffer, and window activation (plan-v0.1.md increment 10,
//! ADR-003). Two listeners carry documents (the single-instance callback, `RunEvent::Opened`) and
//! both terminate in `route_open`; a third (`RunEvent::Reopen`) carries no paths and calls
//! `activate` alone — routing and activation are separate concerns (ADR-003), which only became
//! visible once a listener existed that needed one but not the other.
//!
//! `route_open`, `paths_from_argv`, `paths_from_urls` and `PendingOpens` are plain Rust — no
//! `tauri::` type in sight — which is what makes "must not block" true by the code's shape rather
//! than by a timing test (the same unfalsifiable-property-as-signature move as the lock
//! extraction's owned root, plan-v0.1.md's own note): a function given only owned paths and
//! returning a decision cannot reach a dialog, a walk, or an await on the frontend, so it cannot
//! block regardless of how carefully or carelessly it's called. `activate` is the one Tauri-aware
//! export here, and it contains no decision to extract — three unconditional calls in a fixed
//! order, each present for a stated reason (ADR-003) — which is exactly the shape architecture.md
//! §2 allows in a shell: mechanical action, not a choice.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::Url;

use crate::error::MeddError;

/// What one launch path resolves to, once canonicalised. A directory becomes the workspace root;
/// anything else opens as a document (architecture.md §5) — routing does not restrict documents
/// to `.md`, the same way `document_read` accepts any path (W-8); that filtering, where it
/// matters, is the frontend's per `open:request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RouteTarget {
    Workspace { path: PathBuf },
    Document { path: PathBuf },
}

/// Canonicalises and classifies each incoming path independently, so one bad path doesn't take
/// the rest down with it — the common case being exactly the one architecture.md §9 calls out:
/// `medd <nonexistent-file>` must error, not create, and must not stop any *other* path on the
/// same command line from opening.
pub fn route_open(paths: Vec<PathBuf>) -> Vec<Result<RouteTarget, MeddError>> {
    paths.into_iter().map(route_one).collect()
}

fn route_one(path: PathBuf) -> Result<RouteTarget, MeddError> {
    let canonical = path.canonicalize().map_err(|e| MeddError::io(&path, e))?;
    if canonical.is_dir() {
        Ok(RouteTarget::Workspace { path: canonical })
    } else {
        Ok(RouteTarget::Document { path: canonical })
    }
}

/// `argv[0]` is always the invoked binary's own path — true of `std::env::args()` for the
/// primary's own cold-launch arguments, and true of whatever `tauri-plugin-single-instance`
/// forwards from a secondary launch, since both ultimately come from the same shim executing the
/// same bundle binary (architecture.md §9). Never a user-supplied path, and every listener that
/// reads argv must skip it before anything reaches `route_open` — tested at this seam rather than
/// inside `route_open` itself, since `route_open` never sees an argv at all.
pub fn paths_from_argv(argv: &[String]) -> Vec<PathBuf> {
    argv.iter().skip(1).map(PathBuf::from).collect()
}

/// `RunEvent::Opened` carries `file://` URLs (Finder double-click, "Open With"). Anything that
/// isn't a `file:` URL — or a `file:` URL `to_file_path` can't turn into a path at all, which
/// happens for a bare host component like `file://host/path` — is dropped rather than handed to
/// `route_open` as a bogus path: better to silently drop a scheme medd was never sent in practice
/// than to canonicalise garbage and report a confusing error for a launch the user didn't make.
pub fn paths_from_urls(urls: &[Url]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|url| url.to_file_path().ok())
        .collect()
}

/// One launch's routed opens, plus the frontend-readiness state that decides whether they're
/// delivered live or held. Named for what it does, not for what's inside it (architecture.md §5's
/// naming pattern): the interesting fact is "what should the caller do with this open", and the
/// buffer is an implementation detail of answering that.
struct PendingState {
    ready: bool,
    buffered: Vec<RouteTarget>,
}

/// What a caller should do with a batch of routed targets, decided once by `offer` so the caller
/// (`main.rs`, a shell) only ever matches on the answer rather than computing it — the same split
/// as `QuitCoordinator::decide`/`ExitDecision`.
pub enum Delivery {
    /// The frontend is already listening: emit `open:request` with these now.
    Live(Vec<RouteTarget>),
    /// Held. Nothing further to do until `mark_ready_and_drain` is called.
    Buffered,
}

/// `RunEvent::Opened` can fire before the WebView has attached its listeners — documented
/// behaviour, not an edge case (architecture.md §5) — so opens are buffered here and drained by
/// the frontend's own `frontend_ready` call. A cold launch goes through this buffer every time;
/// it is the normal path, not the exception.
///
/// `ready` is a one-way latch, deliberately: once `mark_ready_and_drain` has been called, every
/// later `offer` delivers live and nothing more is ever appended to `buffered`, which is what
/// makes a *second* `mark_ready_and_drain` call (WebView reload, dev HMR, crash-reload) safe to
/// return empty rather than re-delivering what the first call already returned — QA's criterion
/// that draining must satisfy both "nothing lost" and "nothing duplicated". What this does not
/// solve, and isn't asked to: an open racing the exact window between a reload starting and the
/// new page re-attaching its listener, which is narrower than but the same *kind* of gap as the
/// simultaneous-cold-launch race ADR-003 already accepts rather than papers over.
pub struct PendingOpens {
    state: Mutex<PendingState>,
}

impl PendingOpens {
    pub fn new() -> Self {
        PendingOpens {
            state: Mutex::new(PendingState {
                ready: false,
                buffered: Vec::new(),
            }),
        }
    }

    /// Called by each listener with what it routed. A drop listener (v0.2, ADR-003 §4) would call
    /// this too, and would always see `Live` back — a drop can only happen once the window and
    /// frontend already exist, so it shares this decision point without ever populating the
    /// buffer, deliberately rather than by accident.
    pub fn offer(&self, targets: Vec<RouteTarget>) -> Delivery {
        let mut state = self.state.lock().unwrap();
        if state.ready {
            Delivery::Live(targets)
        } else {
            state.buffered.extend(targets);
            Delivery::Buffered
        }
    }

    /// Marks the frontend ready and returns everything accumulated so far, exactly once per
    /// arrival: anything already drained by an earlier call is gone from `buffered` and cannot
    /// come back, and anything offered after `ready` became true was never buffered to begin with.
    pub fn mark_ready_and_drain(&self) -> Vec<RouteTarget> {
        let mut state = self.state.lock().unwrap();
        state.ready = true;
        std::mem::take(&mut state.buffered)
    }
}

impl Default for PendingOpens {
    fn default() -> Self {
        Self::new()
    }
}

/// `unminimize()` → `show()` → `set_focus()`, unconditionally and in that order (ADR-003). No
/// branch on window state: `tao`'s own `set_focus()` already guards itself on `isMiniaturized()`/
/// `isVisible()`, which is exactly why a minimised or hidden window used to silently skip
/// activation and report success — `unminimize()`/`show()` exist to make those guards pass, not to
/// be guarded themselves. Best-effort and not depended upon: the file opens as a tab whether or
/// not the window comes forward (architecture.md §5), so every error here is swallowed rather than
/// surfaced.
pub fn activate<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn a_directory_becomes_a_workspace_target() {
        let dir = tempdir().unwrap();
        let results = route_open(vec![dir.path().to_path_buf()]);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &RouteTarget::Workspace {
                path: dir.path().canonicalize().unwrap()
            }
        );
    }

    #[test]
    fn a_file_becomes_a_document_target() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "x").unwrap();

        let results = route_open(vec![file.clone()]);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &RouteTarget::Document {
                path: file.canonicalize().unwrap()
            }
        );
    }

    #[test]
    fn a_nonexistent_path_errors_rather_than_being_silently_dropped() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope.md");

        let results = route_open(vec![missing]);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn one_bad_path_does_not_take_the_others_down_with_it() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real.md");
        fs::write(&real, "x").unwrap();
        let missing = dir.path().join("missing.md");

        let results = route_open(vec![real.clone(), missing, dir.path().to_path_buf()]);

        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &RouteTarget::Document {
                path: real.canonicalize().unwrap()
            }
        );
        assert!(results[1].is_err());
        assert_eq!(
            results[2].as_ref().unwrap(),
            &RouteTarget::Workspace {
                path: dir.path().canonicalize().unwrap()
            }
        );
    }

    #[test]
    fn paths_from_argv_skips_only_argv_zero() {
        let argv = vec![
            "/Applications/medd.app/Contents/MacOS/medd".to_string(),
            "/tmp/a.md".to_string(),
            "/tmp/b.md".to_string(),
        ];
        assert_eq!(
            paths_from_argv(&argv),
            vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.md")]
        );
    }

    #[test]
    fn paths_from_argv_on_a_bare_launch_is_empty() {
        let argv = vec!["/Applications/medd.app/Contents/MacOS/medd".to_string()];
        assert_eq!(paths_from_argv(&argv), Vec::<PathBuf>::new());
    }

    #[test]
    fn paths_from_urls_extracts_file_paths() {
        let urls = vec![Url::parse("file:///tmp/a.md").unwrap()];
        assert_eq!(paths_from_urls(&urls), vec![PathBuf::from("/tmp/a.md")]);
    }

    #[test]
    fn paths_from_urls_drops_a_non_file_scheme() {
        let urls = vec![Url::parse("https://example.com/a.md").unwrap()];
        assert_eq!(paths_from_urls(&urls), Vec::<PathBuf>::new());
    }

    #[test]
    fn paths_from_urls_percent_decodes() {
        let urls = vec![Url::parse("file:///tmp/My%20Notes.md").unwrap()];
        assert_eq!(
            paths_from_urls(&urls),
            vec![PathBuf::from("/tmp/My Notes.md")]
        );
    }

    #[test]
    fn a_launch_before_the_frontend_is_ready_is_buffered_not_delivered() {
        let pending = PendingOpens::new();
        let target = RouteTarget::Document {
            path: PathBuf::from("/tmp/a.md"),
        };

        let delivery = pending.offer(vec![target]);
        assert!(matches!(delivery, Delivery::Buffered));
    }

    #[test]
    fn the_awkward_but_real_order_emit_then_attach_then_drain_still_delivers() {
        // The order this test insists on is the point (QA's own framing): a launch can route an
        // open before the frontend has attached anything at all. The natural-looking test —
        // attach first, then emit, then assert — would pass even if this buffer did not exist,
        // because the listener would already be there. This one emits into an empty, unattached
        // state first, and only then "attaches" (calls mark_ready_and_drain, standing in for the
        // frontend's own frontend_ready invoke) to prove the buffer, not a listener, is what
        // carried the open across that gap.
        let pending = PendingOpens::new();
        let target = RouteTarget::Document {
            path: PathBuf::from("/tmp/a.md"),
        };

        let delivery = pending.offer(vec![target.clone()]); // emit, before anything is attached
        assert!(matches!(delivery, Delivery::Buffered));

        let drained = pending.mark_ready_and_drain(); // attach + drain, as one call here
        assert_eq!(drained, vec![target]);
    }

    #[test]
    fn once_ready_a_new_open_is_delivered_live_not_buffered() {
        let pending = PendingOpens::new();
        pending.mark_ready_and_drain();

        let target = RouteTarget::Document {
            path: PathBuf::from("/tmp/a.md"),
        };
        let delivery = pending.offer(vec![target.clone()]);

        match delivery {
            Delivery::Live(targets) => assert_eq!(targets, vec![target]),
            Delivery::Buffered => panic!("expected a live delivery once ready"),
        }
    }

    #[test]
    fn draining_twice_neither_loses_nor_repeats_an_open() {
        // G3: frontend_ready can fire more than once (WebView reload, dev HMR, crash-reload).
        // Both halves matter separately — a buffer that is never cleared would pass "nothing
        // lost" while re-opening every file on every reload, which is exactly the failure this
        // test is written to catch and the naive "assert nothing lost" version would not.
        let pending = PendingOpens::new();
        let target = RouteTarget::Document {
            path: PathBuf::from("/tmp/a.md"),
        };
        pending.offer(vec![target.clone()]);

        let first_drain = pending.mark_ready_and_drain();
        assert_eq!(first_drain, vec![target], "nothing lost");

        let second_drain = pending.mark_ready_and_drain();
        assert_eq!(
            second_drain,
            Vec::<RouteTarget>::new(),
            "nothing duplicated"
        );
    }

    #[test]
    fn opens_offered_between_two_drains_are_not_lost() {
        let pending = PendingOpens::new();
        pending.mark_ready_and_drain(); // first drain, nothing buffered yet

        // Once ready, offer() delivers live rather than buffering (the test above covers that).
        // What this test covers is the OTHER half of "between the drains": a second drain must
        // still faithfully return empty rather than fabricating something, since a ready
        // PendingOpens never buffers again — proving the latch, not just the first transition.
        pending.offer(vec![RouteTarget::Document {
            path: PathBuf::from("/tmp/a.md"),
        }]);

        let second_drain = pending.mark_ready_and_drain();
        assert_eq!(second_drain, Vec::<RouteTarget>::new());
    }
}
