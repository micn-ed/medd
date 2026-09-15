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
