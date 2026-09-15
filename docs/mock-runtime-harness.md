# The mock-runtime harness: no, and why

**From:** principal architect
**Question:** can Tauri's `MockRuntime` give the launch/exit subsystem a deterministic in-process guard, so the window-close-doesn't-flush class of bug has a test rather than a code review?
**Answer:** no, and not for want of setup. Read from `tauri` 2.11.5.
**Status:** bounded investigation, concluded. Recorded so the question stays closed.

---

## The three facts

**1. The mock's loop terminates on exactly one condition.** `MockRuntime::run` breaks only when a
`CloseWindow` or `DestroyWindow` message empties the window map *and* the resulting
`ExitRequested` is **not** prevented. Otherwise it emits `MainEventsCleared`, sleeps a second, and
goes round again.

**2. medd's handler prevents on exactly that path.** That is the fix: `ExitRequested` is prevented,
the flush is asked for, and the process leaves via `AppHandle::exit(0)` once the frontend reports
done or the ceiling expires.

**3. The mock runtime has no `RequestExit`.** Its message enum is three variants —

```rust
enum Message {
  Task(Box<dyn FnOnce() + Send>),
  CloseWindow(WindowId),
  DestroyWindow(WindowId),
}
```

— and `RequestExit` appears nowhere in the file. So `AppHandle::exit()`, which is how medd escapes
its own prevented exit, **does nothing under the mock.**

Put together: driving medd's real exit handling under `MockRuntime` **hangs by construction.** The
loop waits for an unprevented `ExitRequested`; medd prevents it; medd's escape hatch is
unimplemented. There is no arrangement of the test that fixes this, which is why my first attempt
hung — it was not a setup mistake.

That also explains the shape of what *is* reachable. `ExitRequested` can be observed, but only on
the path where the handler lets the exit proceed — which is the path without the flush. The mock
can drive medd's exit handling only in the configuration whose behaviour was never in doubt.

---

## The fallback, and why I am declining to build it

The obvious consolation prize is a **characterisation test**: a test-local handler that records the
event order and asserts `CloseRequested` is delivered *before* the window is destroyed, with
`ExitRequested` following it. That ordering is the exact fact my review turned on — the
window-close bug was that `ExitRequested` fires from `Destroyed`, after the webview is gone, so the
flush request reaches nothing.

It would terminate, it would pass, and it would be worth nothing.

The ordering it pins is **`MockRuntime`'s**. The real ordering is `tauri-runtime-wry`'s, and the two
are separate implementations of the same trait. A green characterisation test against the mock
would say nothing whatsoever about the runtime medd actually ships — while *looking* exactly like a
guard on the premise the design depends on.

That is the pattern this project keeps finding, and it would be its own next instance: a check that
retires a question it cannot answer. `own_write_produces_no_notification` asserted something true
and adjacent; the per-listener criterion asserted something true and adjacent about shaping; this
would assert something true and adjacent about a runtime we do not use. Same failure, third costume.

So the honest position is the one we already have: **the window-close ordering is verified by
reading `tauri-runtime-wry`, and re-verified by reading it again after a Tauri upgrade.** That is
worse than a test. It is also the only thing available, and saying so is better than manufacturing
a green tick.

---

## What this leaves, and where the guard actually is

| Path | In-process guard | What covers it |
|---|---|---|
| The flush decision (what is owed, what is issued, whose outcome applies) | **Yes** — already built | `doc.ts`'s tests, QA's composition suite, the three-clause invariant |
| `QuitCoordinator`'s timing and idempotency | **Yes** — already built | `quit.rs`'s own tests, no runtime needed |
| Which platform hook fires, and when | **No, and not obtainable** | Reading the runtime; the manual pass; the end-to-end gesture |

The middle column is the useful conclusion: **everything about this subsystem that could have a
deterministic guard already has one.** What is left uncovered is exactly the part that is a fact
about macOS and Tauri rather than a fact about medd — and a mock of Tauri cannot testify about
Tauri.

Which is the same division the project arrived at for activation: three named window states with
predicted outcomes, checked by hand, because `set_focus`'s behaviour is not medd's to assert. The
launch and exit hooks are in that category too. The difference is that we now know it rather than
assume it.

---

## One upstream note, if anyone wants it later

The gap is small and specific: `MockRuntime` handling `Message::RequestExit` by emitting
`ExitRequested` and breaking when unprevented would make prevent-then-exit flows testable
in-process, which is the ordinary shape for any app that flushes on quit. That is a plausible
upstream contribution rather than something to carry as a local patch — and it would only ever buy
the *decision* coverage we already have, not the hook-ordering coverage we actually lack. Worth
recording as a real option; not worth doing now.
