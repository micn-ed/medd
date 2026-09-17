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
  lived. The design now removes that site rather than navigating it — if the watch always targets
  the document's own directory, that directory comes from the path and never consults the
  workspace. **The half to check afterwards is the other one:** `scope_and_watch_loose_document`
  does two things, and the asset-protocol grant may still want to know whether the path is inside
  the workspace. If it does, the lock question survives on that half alone, and the plain-`let`
  discipline has to survive with it.

### Raised while the design is open: always-watching has an unbounded set behind it

Today **nothing ever releases a loose document's watch or its scope grant.** `workspace_open`
balances its own pair — `forbid_directory` + `unwatch` on the old root — but `document_read`'s
grant at `commands.rs:193` and its `watch_non_recursive` have no counterpart anywhere: not on tab
close, not on workspace switch, not on quit. Today that is bounded by how many *loose* documents
one session opens, which under D-15 is a handful.

**If the watch becomes unconditional, that set becomes every directory a file was ever opened
from** — and for in-workspace documents each one is **redundant with the recursive root watch that
already covers it.** So the cost is an FSEvents watch and a scope entry per directory visited,
buying nothing, never released, for the lifetime of a session medd is designed to leave running for
days (I-3, N-1).

Not a correctness problem, and I checked: a second watch on the same file does not produce a
duplicate notification, because `check_external_change` updates `last_known` when it reports, so
the second batch finds the hash already current and returns `None`. It is a resource question, and
the answer belongs in the design rather than in a later memory-growth investigation.

**Criterion either way: whatever set the fix grows, say what bounds it.**

### And the scope half is a security boundary, which changes what welding costs

Three documents specify the same thing: the asset protocol is scoped to the workspace root plus
**the directories of *open* loose documents**, "nothing wider" (plan §5), "nothing else is readable
by the WebView" (architecture §11).

**The implementation never revokes.** `allow_directory` at `commands.rs:193` has no counterpart —
close the tab, switch workspace, it stays granted for the life of the process. So the WebView's
readable set is already wider than three documents say, and grows monotonically.

That matters more than the watch half, because **the scope is the only gate.**
`resolveRelativePath` returns an absolute path unchanged and normalises `..`, so a document can
reference any path on disk. Nothing else stops the load: the asset scope decides.

Severity, stated precisely rather than alarmingly: **the exfiltration path is closed.** The CSP is
`img-src 'self' asset: data:` with no remote origin and no outward `connect-src`, so a file loaded
this way cannot leave the machine — it renders on screen and stops there. This is a local-read
boundary being wider than specified, not data theft. Increment 5's own argument is the right frame:
two layers, neither substituting for the other, and this is the first one being loose.

**Where this bears on the ruling:** if scope and watch stay welded and the watch becomes
unconditional, **the scope becomes unconditional with it** — granting the WebView read access to
every directory a file was ever opened from. That turns my earlier resource concern into a
boundary concern, and it is the strongest argument for unwelding them: *which directories may the
WebView load from* and *which directories do we watch* have different correctness criteria, and
only one of them is a security control.

**Criterion: the scope set matches its specification — the directories of *open* loose documents —
or the specification changes deliberately, in all three places, with a reason.** A grant with no
release is the same imbalance as the watch, one layer up, and on the half where it counts.
