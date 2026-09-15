# Increment 10 — the entry points, enumerated rather than assumed

**From:** principal architect
**For:** [plan-v0.1.md](plan-v0.1.md) increment 10, [adr/003-launch-routing.md](adr/003-launch-routing.md), [architecture.md](architecture.md) §5
**Status:** written before implementation. Every claim read from the pinned sources — `tauri` 2.11.5, `tauri-runtime` 2.11.3, `tao` 0.35.3 — and checked against medd at `9f6427a`.

The leader's hypothesis was that this subsystem has twice had more entry points than it looked
like it had, and that a third instance would surprise nobody. **There is a third, it carries file
paths, and nothing handles it.**

---

## 1. The fourth entry point: files dropped on the window

```rust
WindowEvent::DragDrop(DragDropEvent::Drop {
    paths: Vec<PathBuf>,        // real filesystem paths
    position: PhysicalPosition<f64>,
})
```

Verified: `drag_drop_enabled` defaults to `true` (`tauri-utils` config), `tauri.conf.json` does not
override it, and **nothing in medd handles the event** — no `WindowEvent` arm in `main.rs`, and no
`ondrop`/`dragover` anywhere in the frontend.

So dragging `notes.md` onto medd's window today does **nothing at all**. Not an error, not a tab —
nothing.

This is the predicted shape exactly:

- It **carries paths**, like the single-instance callback and `RunEvent::Opened`.
- It **bypasses both of them**. It is neither `argv` nor an Apple Event; it arrives as a window
  event from the webview layer.
- It is a **completely ordinary gesture** for a macOS editor — arguably more natural than the CLI,
  and the one a user reaches for when medd is already open and the file is in a Finder window
  beside it.
- It would be **invisible to every test increment 10 plans to write**, because those exercise the
  CLI and (in v0.2) Finder.

One piece of good news: because `drag_drop_enabled` is `true`, Tauri intercepts the drop, so the
WebView does *not* navigate to the dropped file. The current state is "nothing happens", not a
CSP escape. That is worth knowing, because "nothing happens" is a missing feature and a navigation
would have been a security finding.

**Decision needed from the leader.** Dropping a `.md` file on the window should open it as a tab —
it is one more listener calling the `route_open` that increment 10 is building anyway, so the
marginal cost is small once the router exists. But it is not in L-1…L-4, so it is scope, and scope
is yours. What I would not do is leave it undecided: "I dragged a file onto it and nothing
happened" is a bug report whether or not we consider it a feature, and deciding it now costs a
sentence while deciding it later costs an increment.

If deferred, it should be deferred *in writing* next to the other launch paths, so the next person
enumerating entry points finds four and a note rather than three and a gap.

---

## 2. The exhaustive enumeration

So that "four" is checked rather than assumed. Every mechanism by which a path or an activation can
reach medd, with what is actually registered underneath it.

**Live, carrying paths:**

| # | Entry point | Mechanism | Reaches medd today? |
|---|---|---|---|
| 1 | CLI, and Neovim via D-6 | `tauri-plugin-single-instance` callback — `argv` + `cwd` over `/tmp/com_micned_medd_si.sock` | Increment 10 |
| 2 | Finder double-click, Open With, `open -a … file` | `application:openURLs:` → `RunEvent::Opened { urls }` | Handler in increment 10; delivery needs v0.2's `CFBundleDocumentTypes` |
| 3 | **Drag onto the window** | `WindowEvent::DragDrop(Drop { paths })` | **No — unhandled** |

**Live, activation only (no path):**

| 4 | Dock click, `open -a` on a running instance | `applicationShouldHandleReopen:` → `RunEvent::Reopen` | No — unhandled until increment 10 |

**Registered by `tao` but inert in v0.1, and why:**

| 5 | Handoff / user activity | `application:continueUserActivity:restorationHandler:` and `application:willContinueUserActivityWithType:` — both registered by `tao` | Inert: requires `NSUserActivityTypes` in `Info.plist`, which medd does not declare |
| 6 | Services menu | `PredefinedMenuItem::services()` is in medd's menu | Inert as an *inbound* path: a service *sending* to medd needs `NSServices` in `Info.plist`, undeclared. The menu item only offers the system's services for medd's own selection |

**Registered by nothing, which is itself a finding already recorded:**

| 7 | Cmd+Q / `[NSApp terminate:]` | `applicationShouldTerminate:` — **`tao` does not implement it** | This is why Quit is a custom menu item calling `app.exit(0)` |

The full list of delegate methods `tao` registers is seven: `didFinishLaunching`,
`willTerminate`, `openURLs`, `willContinueUserActivityWithType`, `continueUserActivity`,
`shouldHandleReopen`, `supportsSecureRestorableState`. Anything not on that list cannot reach medd
at all, whatever `Info.plist` says — which is the fact that makes this enumeration closed rather
than merely long.

---

## 3. The verification criterion, which matters more than the enumeration

**Both previous bugs in this subsystem were in the wiring, not the router.** Cmd+Q never reached
the hook; window-close reached it too late. In both cases the thing being called was correct, and
in both cases a test of that thing would have passed.

Increment 10 will make `route_open(paths)` unit-testable at the seam — canonicalisation,
classification, `argv[0]` skipping, `file:` filtering — and every listener will call it. **A green
`route_open` suite is precisely what "covering one is indistinguishable from covering all" looks
like.** It tests the one part of this subsystem that has never been broken.

So the verification has to be per-listener, and it has to observe something the *router* did rather
than something the listener did. Concretely, the criterion I would hold increment 10 to:

> **For each listener, break only that listener's call into the router, and confirm that exactly
> one test fails, and that it is the test named for that listener.**
>
> - Zero tests fail → that listener is untested, whatever the suite's coverage says.
> - Tests named for a *different* listener fail → the tests do not distinguish the routes, and
>   covering one really is covering all.

That is QA's mutation discipline applied per entry point rather than per assertion, and it is
checkable mechanically. It is also the only form of verification that would have caught either of
the two bugs we already found here.

The `route_open` unit tests are still worth having. They are just not evidence about the thing that
keeps breaking.

---

## 4. Two smaller notes

**A drop listener would need `route_open` to be reentrant-safe against the pending-open buffer.**
Unlike the other listeners, a drop can only happen when the window exists and the frontend is
ready, so it never needs buffering — but it will share the code path that does. Worth being
deliberate that the buffer is *skipped* rather than accidentally populated-and-drained on a path
where the frontend is already listening.

**One low-probability interaction, named rather than asserted.** `drag_drop_enabled: true` installs
Tauri's own drag handler on the webview, and Tauri's config documents that disabling it "is
required to use HTML5 drag and drop on the frontend **on Windows**". So the interference is
documented as Windows-specific and macOS is probably unaffected — but CodeMirror uses HTML5
drag-and-drop for dragging selected text to a new position, which E-6's "standard editing
affordances" implies medd should have, and nobody has looked. One drag in increment 12's manual
pass settles it. I am flagging it at its real weight: probably nothing, cheap to check, and the
kind of thing that is annoying to discover after shipping.
