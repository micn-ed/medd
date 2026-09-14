# Q-10 — How does a second launch reach the running instance?

**Question restated.** medd must run as a single process (D-7). CLI invocation, Finder
double-click / "Open With", and a future Neovim `:Medd` command (D-6) must all deliver a path to
the one running instance and bring its window forward, rather than starting a second process.
One mechanism should serve all three, and the mechanism has to survive the fact that a running
window may be behind other apps, minimized, or (per requirements) simply not frontmost.

## What Tauri's single-instance plugin actually does today

`tauri-plugin-single-instance` is at 2.4.3 (crates.io, released 2026-07-13), against Tauri core
2.11.5 (2026-07-01). The plugin's history matters here because its macOS story is younger and
shakier than its Windows/Linux story, and the requirements doc explicitly calls that out as a
risk worth checking rather than assuming.

For a long time the plugin simply did not support macOS — tracked in
[plugins-workspace#287](https://github.com/tauri-apps/plugins-workspace/issues/287). A first
attempt to add it ([PR #749](https://github.com/tauri-apps/plugins-workspace/pull/749)) used a
FIFO (named pipe) and pulled in tokio as a dependency; it was rejected in review. The version
that actually shipped ([PR #1035](https://github.com/tauri-apps/plugins-workspace/pull/1035),
merged 2024-03-27) uses a Unix domain socket at a well-known path instead, with no extra async
runtime dependency, and this is what `tauri-plugin-single-instance` does on macOS today: the
first instance binds a Unix socket, later invocations detect it, write their `argv` and cwd to
it, and exit. The plugin's `init()` callback on the running instance receives those forwarded
arguments and is where you're expected to call `window.set_focus()` yourself — the plugin does
not raise the window for you.

That focus call is the weak link. Two open issues describe it failing under exactly the
conditions medd cares about: [tauri#12936](https://github.com/tauri-apps/tauri/issues/12936)
reports `set_focus()` not working when the window was previously hidden, and
[plugins-workspace#1613](https://github.com/tauri-apps/plugins-workspace/issues/1613) — closed
as "not planned" — confirms the callback fires but the resulting focus attempt is unreliable on
macOS specifically when the window is hidden rather than minimized; the workaround offered was
"minimize instead of hide," which trades one piece of undesired behavior for another. This isn't
really a Tauri bug so much as it is macOS: `NSRunningApplication.activateWithOptions` /
`NSApplication.activate(ignoringOtherApps:)` — the underlying call TAO (Tauri's windowing layer)
uses — has been documented by Apple developers as unreliable since Big Sur and is now deprecated
on Sonoma, sometimes silently declining to activate an app that isn't already somewhat "eligible"
for activation. This is a platform limitation the plugin inherits, not something a different IPC
choice would fix.

Separately, and this is the part of the question most worth getting right: the single-instance
socket only ever carries **CLI-shaped argv**. Finder double-click and "Open With" do not go
through argv at all on macOS. They arrive as an Apple Event that macOS delivers to the *already
running* app via the `application:openFiles:` / `application:openURLs:` delegate methods (or, if
the app isn't running yet, as part of a normal cold launch that Tauri surfaces as
`RunEvent::Opened`, containing the file paths/URLs). Tauri 2.0 wires this up for deep links and
file associations (`RunEvent::Opened`, added alongside iOS support — see
[tauri@753900d](https://github.com/tauri-apps/tauri/commit/753900dd6e549aaf56f419144382669e3b246404)
and the file-associations example in the Tauri repo), but it is a **completely separate event
path from the single-instance plugin's socket**. A Finder double-click on a second `.md` file,
while medd is already running, does not touch the single-instance socket at all — it lands on
the running process's `RunEvent::Opened` handler directly, because macOS itself routes it there
via Apple Events. There is no "second process" in that path to detect and hand off from; the OS
already did the routing. The practical consequence is that medd needs two listeners feeding one
router, not one universal mechanism: the single-instance socket for argv-shaped launches (CLI,
and — once it exists — Neovim, since D-6 has Neovim call the same entry point as the CLI), and
`RunEvent::Opened` for Finder/`open`-shaped launches, both terminating in the same "open this
path as a tab and raise the window" function.

## Alternatives considered

**Roll a bespoke Unix domain socket instead of the plugin.** Functionally this is what the
plugin already does internally, so building it by hand buys nothing except full control over the
wire format and error handling — useful if the plugin's argv-only framing turns out to be
limiting, but not worth the maintenance cost up front. Worth keeping as a fallback if the plugin
proves too opaque to debug.

**macOS distributed notifications (`NSDistributedNotificationCenter`).** A genuinely
macOS-native broadcast mechanism — any process can post a notification by name and any other
process can observe it, with no socket file to manage or clean up. It's a reasonable way to
*wake* a running instance, but it's a fire-and-forget pub/sub primitive: payload size is limited,
delivery isn't guaranteed, and there's no built-in "is anyone listening" check, which matters for
detecting whether an instance is running at all before deciding whether to become the primary.
It would need to be paired with something else (a lock file, or checking for the socket) purely
to answer "am I the first instance," which erases most of its simplicity advantage over just
using a socket for everything.

**Lock file + local TCP port.** Works, and is platform-portable (relevant given N-4's "don't
gratuitously block Linux later"), but a TCP port on localhost is one more thing to firewall-check
and one more number to collide with other local dev servers. A Unix domain socket gets the same
IPC semantics with filesystem-scoped naming and permissions, and Tauri already ships exactly this
for the CLI path — there's no reason to reimplement it as TCP.

**`open -a Medd --args <path>`** as the CLI's actual implementation (i.e., shell out to `open`
rather than talking to the socket directly). This is tempting because `open` is what already
knows how to find the `.app`, launch it if needed, or hand args to it if not — but `open -a`'s
argument-passing behavior for an *already-running* app is inconsistent (it's designed for launch
arguments, not for reliably reaching a live process), and it reintroduces the Apple-Events path
for what is conceptually a CLI, argv-shaped request. It's better kept as a fallback launcher of
last resort, not the primary channel.

## Recommendation

Use `tauri-plugin-single-instance` (Unix-domain-socket backed, current on macOS) for the CLI and
Neovim paths, and Tauri's native `RunEvent::Opened` for the Finder/"Open With" path, both feeding
one internal `route_open(path: PathBuf)` function that opens-or-focuses a tab and then attempts
window focus. Do not build a custom socket; the plugin already is one, and reinventing it only
costs maintenance for no capability gain. Treat window activation as best-effort: call
`set_focus()` (or `show()` + `set_focus()`), but don't design any behavior that depends on it
succeeding every time — worst case on a flaky macOS activation, the file still opens as a tab in
the existing window, which the user can then click to bring forward themselves. This is
consistent with how both open Tauri issues resolved: there's no clean fix upstream, only a
degraded-gracefully posture downstream.

For the startup race — two launches within the same instant, before either has bound the
socket — the plugin's OS-level socket bind is naturally exclusive: whichever process binds first
wins and becomes primary, and the loser's connection attempt to a socket with no listener yet
will simply fail to connect within its retry window. The practical fix is a short retry-with-
backoff (e.g. three attempts over ~100ms) on the client side before falling back to "become
primary myself" — an extremely unlikely path in a single-user desktop app, but cheap to guard
against a doubled-instance in the pathological case of both processes racing to bind
simultaneously (only one bind can succeed; the loser retries as a client).

The CLI-binary-vs-.app-bundle question is Q-14's to answer, but it touches this decision at one
point worth flagging now: whatever `medd` on `PATH` turns out to be — a copy of the same binary
that also runs inside `Contents/MacOS`, or a thin shim — it must be able to find and connect to
the same socket path the running `.app` instance bound. That argues for the socket path being
derived from something stable and shared (e.g. a fixed path under `~/Library/Application
Support/medd/` or `$TMPDIR`) rather than anything computed from the invoking binary's own
location, so the two don't need to agree on where they live relative to each other — only on the
one path they both compute the same way.

## Trade-off accepted

Two listeners instead of one clean universal mechanism, because the OS itself hands Finder-origin
opens to a different delegate than CLI-origin opens — there's no way to unify them at the
transport level without fighting Tauri's own event model. And window activation is accepted as
best-effort rather than guaranteed, because the underlying macOS API it depends on
(`NSRunningApplication.activateWithOptions`) is itself unreliable and deprecated as of Sonoma,
independent of anything medd or Tauri does.

## Risks / unknowns

The `set_focus()` reliability issue ([tauri#12936](https://github.com/tauri-apps/tauri/issues/12936))
is open and unresolved upstream as of the versions checked; it should be spot-tested early against
whatever the actual window state is after autosave/idle (hidden vs minimized vs behind other
windows), since the failure mode reported is specific to `hide()`-style states. It's not yet
confirmed whether medd's window will ever be in a `hidden` (vs merely occluded) state in practice
— if it never calls `hide()`, this risk may not materialize at all, but it's worth an explicit
manual test once the app exists rather than assuming. Separately, `RunEvent::Opened`'s ordering
relative to window/frontend readiness (it can fire before the frontend has attached listeners,
per Tauri's own examples) means the "open this path" payload needs to be buffered in Rust-side
state and pulled by the frontend on ready, not fired-and-forgotten as a one-shot event — this is
a real implementation detail for whoever builds this, not just a doc footnote.

## Sources

- [Single Instance plugin docs](https://v2.tauri.app/plugin/single-instance/) — Tauri v2, current as fetched Sept 2026
- [tauri-plugin-single-instance on crates.io](https://crates.io/crates/tauri-plugin-single-instance) — 2.4.3, released 2026-07-13
- [Tauri core releases](https://v2.tauri.app/release/tauri/) — 2.11.5, released 2026-07-01
- [plugins-workspace#287 — will single-instance support macOS?](https://github.com/tauri-apps/plugins-workspace/issues/287)
- [plugins-workspace#749 — rejected FIFO-based macOS PR](https://github.com/tauri-apps/plugins-workspace/pull/749)
- [plugins-workspace#1035 — merged Unix-domain-socket macOS implementation](https://github.com/tauri-apps/plugins-workspace/pull/1035) (merged 2024-03-27)
- [tauri#12936 — set_focus() broken via single-instance plugin when window was hidden](https://github.com/tauri-apps/tauri/issues/12936)
- [plugins-workspace#1613 — single-instance callback doesn't fire after hide(), closed not-planned](https://github.com/tauri-apps/plugins-workspace/issues/1613)
- [tauri@753900d — RunEvent::Opened exposed for iOS/deep-link/file-open support](https://github.com/tauri-apps/tauri/commit/753900dd6e549aaf56f419144382669e3b246404)
- [Tauri file-associations example](https://github.com/tauri-apps/tauri/tree/dev/examples/file-associations)
- [Apple Developer Forums — NSRunningApplication activateWithOptions unreliable since Big Sur](https://developer.apple.com/forums/thread/668913)
- [Apple Developer Forums — activateWithOptions does not work on Sonoma](https://developer.apple.com/forums/thread/739524)
