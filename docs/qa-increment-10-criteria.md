# Increment 10 — acceptance criteria, written before the code

**From:** QA
**For:** [plan-v0.1.md](plan-v0.1.md) §10 and [review-increment-10.md](review-increment-10.md)
**Status:** criteria fixed in advance.

The architect's review establishes what is true of the platform and where the design is silent.
This is the other half: which of §10's claims a plausible test would pass **without checking**, and
which cannot be tested at all and therefore need naming rather than assuming. It does not restate
any of F1–F9 or G1–G6.

---

## 1. The pending-open buffer is the easiest thing here to test vacuously

§10 says a cold launch goes through the buffer **every time** — "it is the normal path, not the
exception." That makes it the most load-bearing mechanism in the increment, and the natural test
does not exercise it at all:

```
attach listeners  ->  emit an open  ->  assert the tab opened
```

That passes with no buffer whatsoever, because the listener was already there. *Would this still
pass if the mechanism never ran?* Yes. The buffering is entirely unexercised by the obvious test,
and the obvious test is the one that gets written, because it reads as "opening a file works".

**The discriminating order is the inconvenient one:** emit the open **before** `frontend_ready()`,
then attach, then drain — and assert it still arrives. The test has to be written in the awkward
sequence precisely because the awkward sequence is the real one.

**And G3's case needs its own test, not a note.** `frontend_ready()` firing twice — WebView reload,
dev HMR, crash-reload — must not lose opens that arrived between the drains, and must not re-deliver
ones already drained. Both halves: a test that only checks "nothing is lost" passes against an
implementation that never clears the buffer and re-opens every file on each reload.

---

## 2. "Never start a second process" cannot be established by a unit test

I-1 and L-4 are process-level claims. A test of the socket-binding logic proves the socket-binding
logic; it says nothing about whether two invocations converge, which is the actual requirement.
This is the same decomposition as the quit gesture, and worth stating the same way so the covered
part is not mistaken for the whole:

| link | what can establish it |
|---|---|
| the shim picks the right binary and argv | shell-level test of `scripts/medd` |
| a second bind is refused and forwards instead | Rust unit test |
| **both invocations converge on one process** | **two real processes — nothing smaller** |
| the forwarded paths become tabs | frontend test against `open:request` |

The third row is the requirement. **Criterion: one integration check that runs `medd a.md` then
`medd b.md` and asserts exactly one process and two tabs.** It is keyboard-only and needs no GUI
driving, which is what makes it cheap here and impossible before this increment.

---

## 3. Relative path resolution must be tested through the shim, not through `route_open`

G5 settles that the shim is authoritative. The consequence for testing: a Rust test of
`route_open` that passes absolute paths — which every existing path-handling test does, because
`canonicalize()` demands they exist — never exercises resolution at all.

**Criterion: the test runs the shim from a different working directory than the workspace**, with a
relative argument, and asserts the absolute path that arrives. "The running instance's working
directory is not the user's" is a claim about two processes disagreeing; a single-process test
cannot fail it.

---

## 4. "`route_open` must not block" is the quick-open bullet again

G4 states the constraint correctly and it has the same problem as §9's "must not block the dialog
opening": no observable, no threshold, and no test can fail it. It is also the same *kind* of
property — about what the code may contain rather than what it computes — so it wants the same
treatment: **verified structurally, by the function's shape, not by a timing test.** No dialog, no
walk, no await on the frontend, asserted by the fact that `route_open` cannot reach anything that
does those things. If it takes only owned paths and returns a decision for the caller to act on,
the constraint holds by construction — the same move as the lock extraction's owned root.

---

## 5. Two things worth taking while the CLI makes them cheap

Both become keyboard-only reachable for the first time, and both are data-loss adjacent rather
than convenience:

- **Quit with several dirty tabs**, opened by repeated `medd <file>` into the running instance.
  Criterion 2 of §13 is unit-verified and has never run against real files; multi-tab scale is what
  made the quit flush a blocker rather than a repeat of the single-tab case.
- **Quit with a conflicted tab open** — modify externally, let the banner appear, quit inside the
  debounce window. The file must still hold the external change. Verified in Rust, never against a
  real gesture, and it is the assertion that would catch a regression into "just force it through".
