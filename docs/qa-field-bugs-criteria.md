# Acceptance criteria for the two field bugs — written before the code

**From:** QA
**For:** the reload bug (two unrelated causes) and the tree-never-updates bug
**Status:** criteria fixed in advance, while both designs are still with the architect.

Written now rather than at verification time, for the reason that has held five times on this
project: criteria written afterwards get fitted to whatever landed.

---

## 1. The tree fix must distinguish "the tree reloaded" from "a message was sent"

**This is the one to get right**, because that confusion is what produced the bug. The backend
emits `tree:changed` and a backend test asserts it does. Nothing in `src/` listens. A fix that
adds a listener, plus a test that asserts the listener ran, repeats the original mistake one layer
further in — and passes.

**The vacuous forms**, all of which will look like coverage:

- asserting `listen('tree:changed', …)` was registered — passes with an empty handler
- asserting the handler was called — passes with an empty handler
- asserting `dir_list` was invoked again — proves a refetch, not a refresh; the tree could discard
  the result
- asking the tree for its contents after firing the event, **without changing the underlying
  fixture** — and this is the subtle one

That last deserves stating plainly: **if the data does not change between the first render and the
event, a successful reload and a no-op are indistinguishable.** Both leave the same list on screen.
A test written that way asserts that the tree still shows what it showed, which is true of a fix
and equally true of nothing at all.

**The discriminating shape:**

1. render the tree; assert it shows exactly `[a.md]`
2. **change the fixture** so `dir_list` would now return `[a.md, b.md]`
3. assert the tree *still* shows `[a.md]` — establishing that nothing refreshes on its own, so
   step 5 cannot be satisfied by an unrelated re-render
4. fire `tree:changed`
5. assert the tree now shows `[a.md, b.md]`

Step 3 is the half that is easy to drop and is what makes steps 4–5 mean anything. It is the same
structure as the quick-open cache criterion — *establish that a stale result would otherwise be
served* — and for the same reason.

**The instrument exists now and did not before.** `eventMock.ts` retains handlers and exposes
`window.medd.emit`, so the harness can drive this end to end against a real rendered tree. This is
the first bug where that fix pays for itself; before it, this test could only have been written in
jsdom, which lays nothing out and would have made "the tree shows" unassertable.

**If the event is renamed** (the naming question is with the architect — an event named for an
effect its emitter cannot produce makes upstream tests read as end-to-end ones), the criteria are
unchanged. The test still has to observe the tree's contents. A better name removes the *invitation*
to the mistake; it does not remove the need to assert against the consequence.

---

## 2. `tree:changed` has two obligations and only one was met

The event is **not** emitted into nothing — `FileIndex::invalidate()` on the following line is a
real consumer that works. So the fix must not break that while adding the second consumer.

**Criterion: after the fix, invalidating the quick-open index and refreshing the tree are both
still driven by the same signal, and each has a test that fails independently of the other.** A
mutant that disables one must not be caught only by a test for the other — that is the situation
the project has already met, where a property was covered but the map of what covered it was
wrong.

---

## 3. The reload bug's first cause: a file opened without a folder is never watched

Dev has committed a characterisation test at `93f9d9d`, with a control, because
`assert!(events.is_empty())` passes just as happily when the harness is broken. **When the fix
lands the first arm goes red; that is the fix, not breakage**, and the test's own message says so
and says to re-aim it.

**Criteria for the fix:**

- the watch is established because the *path* warrants it, not because a workspace happens to be
  open. `is_loose` was false because the workspace `Option` was `None` — a property of application
  state, not a judgement about the file
- a file opened with **no folder open at all** is watched, and an external write to it produces
  exactly one notification with the correct content
- opening a folder afterwards does not unwatch it
- the control arm still fails when the harness is broken — check it was kept, not just that the
  main arm flipped

**And the eliminated theory stays eliminated.** Atomic-rename was ruled out by measurement. The
existing test's comment named *Neovim* and wrote in place, which Neovim does not do — a test that
named the editor from the report and exercised a strategy that editor never uses. **Criterion: any
test claiming to cover an editor's save strategy performs that strategy**, or names no editor.

---

## 4. What I will check that is not in either design

- **Both fixes ship in one dmg.** So a green suite after the merge says nothing about which fix is
  responsible for which behaviour. I will verify each independently, against its own criterion,
  before looking at the pair.
- **The `is_loose` fix touches `document_read`**, which is where the lock-across-syscall hazard
  lived and where the workspace guard is copied out. `no lock is held across a filesystem or OS
  call` is an architecture invariant now; a new watch registration is exactly the kind of OS call
  that would violate it. I will check the guard is still released before the call.
