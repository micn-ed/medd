# ADR-003 — Three listeners, one router, one activator

**Status:** Accepted, 2026-09-14
**Resolves:** [open-questions.md](../open-questions.md) Q-10 (blocks v0.1)
**Evidence:** [research/q10-single-instance-ipc.md](../research/q10-single-instance-ipc.md)
**Context:** D-6 (Neovim hand-off), D-7 (single instance), L-1…L-4, I-1

## Decision

medd uses **three** launch listeners feeding **one** router and **one** activator:

- **`tauri-plugin-single-instance`** (Unix-domain-socket backed, current on macOS) for
  `argv`-shaped launches: the CLI, and by extension Neovim's `:Medd`, which D-6 routes through the
  same entry point.
- **Tauri's `RunEvent::Opened`** for Apple-Event-shaped launches: Finder double-click and
  "Open With".
- **Tauri's `RunEvent::Reopen`** (from `applicationShouldHandleReopen:`) for launches that carry no
  document at all: a Dock click, or `open -a` on an already-running app.

**Routing and activation are separate concerns**, and the third listener is what makes that
visible. `Reopen` carries no paths, so it cannot call the router — which is the tell that activation
was never part of routing in the first place. It was folded in because, with only two listeners,
every launch happened to carry a document.

| Listener | Shape | Carries a document? | Calls |
|---|---|---|---|
| single-instance callback | argv + cwd | usually | `route_open` if any paths, then `activate` |
| `RunEvent::Opened` | Apple Event | always | `route_open`, then `activate` |
| `RunEvent::Reopen` | Apple Event | never | `activate` only |

Separating them also disposes of an awkward case rather than adding one: bare `medd` while an
instance is running yields zero paths after `argv[0]` is skipped, and with activation extracted
there is no empty-path call into the router to define.

Both terminate in a single Rust `route_open(paths)` function. Window activation is attempted and
**not depended upon**.

## Why two

L-4 asks that all three launch paths converge on the running instance, and the natural reading is
that one mechanism should carry all of them. macOS makes that impossible at the transport layer.

A CLI invocation starts a second process with `argv`. The single-instance plugin's client detects
the bound socket, forwards `argv` and the working directory, and exits. That is the whole
mechanism, and it is the right one.

A Finder double-click does not start a second process at all. macOS delivers it to the
*already running* application as an Apple Event, which Tauri surfaces as `RunEvent::Opened`. There
is no second process to detect and no socket traffic to observe — the OS has already done the
routing. Nothing medd does can make that arrive over the same channel as `argv`.

So convergence happens one layer up, in `route_open()`, rather than at the transport. The framing
in open-questions.md — "one mechanism should serve all three" — is therefore corrected to **two
listeners, one router**.

## Why the plugin rather than a bespoke socket

A hand-rolled Unix domain socket is functionally what the plugin already is internally. Building
it by hand buys control over the wire format and error handling, and costs ongoing maintenance for
no capability gain. It is worth keeping as a fallback if the plugin proves too opaque to debug,
not as the starting point.

**Rejected: distributed notifications** (`NSDistributedNotificationCenter`) — genuinely native and
socket-free, but fire-and-forget with limited payload, no delivery guarantee, and no way to ask
"is anyone listening", which is the question that decides whether to become the primary instance.
Pairing it with a lock file to answer that erases its simplicity advantage.

**Rejected: lock file plus localhost TCP** — works and is more portable, but a TCP port is one more
thing to collide with local dev servers and to explain to a firewall, for IPC semantics a Unix
socket already provides with filesystem-scoped naming and permissions.

**Rejected: `open -a Medd --args <path>` as the primary channel** — `open`'s argument passing to an
*already running* app is inconsistent by design (it is a launch mechanism, not an IPC one), and it
reintroduces the Apple-Events path for what is conceptually an `argv`-shaped request. It survives
only as the cold-launch branch of the CLI shim (architecture.md §9), which is exactly what it is
good at.

## Trade-off accepted, and the honest bit

An earlier version of this section said window activation was "best-effort, not guaranteed" and
attributed that to a deprecated macOS API — a platform limitation no IPC choice could fix. **That
was right for one window state out of three, and describing all three that way would have shipped
a fixable bug as an accepted cost.** Honest-sounding text that stops people investigating is its
own hazard.

Traced through `tao` 0.35.3 (`platform_impl/macos/window.rs`), the deprecated call is reachable
*only* via `set_focus()`, and `set_focus()` guards itself:

```rust
pub fn set_focus(&self) {
    let is_minimized = self.ns_window.isMiniaturized();
    let is_visible = self.ns_window.isVisible();
    if !is_minimized && is_visible {
      util::set_focus(&self.ns_window);   // makeKeyAndOrderFront: + activateIgnoringOtherApps:
    }
}   // no else — Window::set_focus() returns Ok(()) either way
```

So in the two states where activation is most needed it is not unreliable; it is **skipped
deterministically, and reports success.**

| Window state | Why activation fails | Fixable here? |
|---|---|---|
| Visible but occluded | the deprecated API genuinely is unreliable | **No.** This is the real platform limitation. |
| **Minimized** (Cmd+M) | `tao`'s `isMiniaturized()` guard skips the call entirely | **Yes** — `unminimize()` first |
| **Hidden** (Cmd+H) | the same guard, via `isVisible()` | **Probably** — `show()` first; to be measured, not assumed |

**`activate()` is therefore `unminimize()` → `show()` → `set_focus()`**, in that order, each step
present for a stated reason. `show()` is only `makeKeyAndOrderFront:`; whether that clears
`isMiniaturized` on a miniaturised window is version-dependent folklore, while `deminiaturize:` is
the documented way to restore one. The general rule: **prefer the call whose behaviour is
documented over the call that happens to work.**

**Do not branch on `Reopen`'s `has_visible_windows` flag.** It is the tempting input and the wrong
one — the CLI path carries no such flag, so branching on it would fix the Dock click while leaving
`medd notes.md` broken when minimised, which is the worst kind of partial fix: it removes the bug's
most reproducible trigger without removing the bug. Query `is_minimized()`/`is_visible()` on the
window instead; that works identically for every caller. Keep the flag for diagnostics.

**The claim that "medd may never enter a `hide()` state" is withdrawn.** It reasoned about what
medd does rather than what medd ships. `main.rs` registers no custom menu, so Tauri's
`Menu::default` applies — which puts **Cmd+H** (`hide`), `hide_others` and **Cmd+M** (`minimize`)
under medd's own application menu. The hidden state is one keystroke away via the most common
"get out of my way" gesture on the platform.

The residual unknown is specific and should be tested rather than predicted: Cmd+H is
`[NSApp hide:]`, which hides the *application*, not the window, and a window-level
`makeKeyAndOrderFront:` may not clear an application-level flag. Its documented counterpart is
`[NSApp unhide:]`, which Tauri does not expose. If `show()` proves insufficient, the choice is an
`objc2` call or accepting the hidden case as genuinely unfixable — a decision to make against a
measurement.

**For the one state that stays unreliable, the posture stands:** the file opens as a tab whether or
not the window comes forward, and no behaviour may assume the window is frontmost after a routed
open. `is_focused()` exists, so increment 12's manual pass should *assert* activation in each of
the three named states with a predicted outcome, rather than eyeballing it — a test with a
prediction catches a wrong prediction; one without catches only a crash.

## Consequences

- **Opens must be buffered.** `RunEvent::Opened` can fire before the WebView has attached its
  listeners — documented behaviour, not an edge case. Opens are pushed to a Rust-side pending
  buffer and drained by the frontend's `frontend_ready()` call. Cold launch from Finder goes
  through this buffer *every* time; it is the normal path.
- **The socket path must be stable and shared.** The CLI shim and the running app must compute the
  same path from nothing about the invoking binary's own location — the shim and the bundle do not
  know where the other lives. **The path itself is the plugin's, not medd's**: it hardcodes
  `/tmp/<identifier>_si.sock` with no configuration hook, so it derives from `config.identifier`, a
  compile-time constant. That already satisfies the invariant. An earlier version of this ADR and
  of architecture.md §9 specified a location under the application support directory, which nothing
  binds — the intent was right and the description was wrong.
- **Startup race — accepted, unmitigated, and worse than first described.** This ADR previously
  claimed the OS-level bind is exclusive so the loser could retry-with-backoff. Neither half is
  implementable or accurate. There is no hook to retry from: the plugin makes one attempt, inside
  `.setup()`. And the bind is not the exclusive step — on `NotFound` the plugin unlinks the socket
  path and *then* binds, so two simultaneous launches both unlink and both bind. Verified: two
  listeners, the first orphaned on an unlinked inode and permanently unreachable, every later
  invocation reaching the second. The visible result is **two windows, and I-1 silently violated**.

  Vanishingly unlikely in single-user desktop use, and not fixable at medd's layer without
  replacing the plugin. It is recorded as a known limitation rather than papered over: a
  requirement marked Must that cannot be fully guaranteed deserves to be stated.
- **Finder registration is v0.2** (scope-mvp.md), but `RunEvent::Opened` handling is designed in
  now so that adding the file association is a manifest change rather than an architectural one.
