# ADR-003 — Two listeners, one router, best-effort activation

**Status:** Accepted, 2026-09-14
**Resolves:** [open-questions.md](../open-questions.md) Q-10 (blocks v0.1)
**Evidence:** [research/q10-single-instance-ipc.md](../research/q10-single-instance-ipc.md)
**Context:** D-6 (Neovim hand-off), D-7 (single instance), L-1…L-4, I-1

## Decision

medd uses **two** launch listeners feeding **one** router:

- **`tauri-plugin-single-instance`** (Unix-domain-socket backed, current on macOS) for
  `argv`-shaped launches: the CLI, and by extension Neovim's `:Medd`, which D-6 routes through the
  same entry point.
- **Tauri's `RunEvent::Opened`** for Apple-Event-shaped launches: Finder double-click and
  "Open With".

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

**Window activation is best-effort, not guaranteed.** `set_focus()` on macOS is unreliable:
`NSRunningApplication.activateWithOptions` — the call TAO ultimately makes — has been documented as
flaky since Big Sur and is deprecated as of Sonoma, sometimes silently declining to activate an app
not already "eligible". Two upstream issues trace to this (`tauri#12936`;
`plugins-workspace#1613`, closed *not planned*, with "minimize instead of hide" offered as a
workaround that trades one undesired behaviour for another).

This is a macOS platform limitation that no IPC choice fixes. medd's posture is to degrade
gracefully: **the file opens as a tab whether or not the window comes forward.** No behaviour in
the design may assume the window is frontmost after a routed open — worst case, the user clicks the
window themselves and their file is already there. This should be spot-tested early against real
window states, since the reported failure is specific to `hide()`-style states and medd may never
enter one.

## Consequences

- **Opens must be buffered.** `RunEvent::Opened` can fire before the WebView has attached its
  listeners — documented behaviour, not an edge case. Opens are pushed to a Rust-side pending
  buffer and drained by the frontend's `frontend_ready()` call. Cold launch from Finder goes
  through this buffer *every* time; it is the normal path.
- **The socket path must be stable and shared.** The CLI shim and the running app must compute the
  same path from the same fixed location (under the application support directory), never from the
  invoking binary's own location — the shim and the bundle do not know where the other lives. This
  is the invariant that Q-14's CLI-distribution design must not break (architecture.md §9).
- **Startup race.** Two simultaneous launches resolve naturally: the OS-level socket bind is
  exclusive, so whichever binds first becomes primary. The loser should retry-with-backoff (three
  attempts over ~100 ms) before falling back to becoming primary itself. Vanishingly unlikely in a
  single-user desktop app, cheap to guard.
- **Finder registration is v0.2** (scope-mvp.md), but `RunEvent::Opened` handling is designed in
  now so that adding the file association is a manifest change rather than an architectural one.
