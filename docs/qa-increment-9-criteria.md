# Increment 9 — acceptance criteria, written before the code

**From:** QA
**For:** [plan-v0.1.md](plan-v0.1.md) §9 and [increment-9-notes.md](increment-9-notes.md), both in flight
**Status:** criteria fixed in advance, so they are not fitted to whatever lands.

The architect's notes cover what quick-open needs to *be*: a shared ignore predicate, an async
walk command, cache invalidation from `tree:changed`, and an `is_markdown` filter. This covers how
we will know it is right, and deliberately does not restate any of that. Everything below is
either something those notes do not reach, or something a plausible test of them would pass
without checking.

---

## 1. "Must not block the dialog opening" is unfalsifiable as written

This is the bullet the whole increment hangs on, and as phrased no test can fail it. "Does not
block" has no threshold and no observable. It is the same shape as increment 10's end-to-end
deliverable, which specified a sequence but not the timing constraint the sequence depended on,
and was therefore green against a quit flush that had been deleted.

It needs two separate checks, because it is two separate claims:

- **The dialog is usable before the walk finishes.** Observable in a frontend test with a walk
  promise that has not resolved: the dialog renders, accepts keystrokes, and shows its empty or
  partial state. Assert against a *pending* promise — a test that awaits the walk first proves
  nothing about ordering.
- **The walk does not stall the UI thread.** Not observable from a unit test at all: whether the
  command is `#[tauri::command(async)]` is a macro attribute, not a behaviour a test can see. It
  needs a number and a real measurement — *the dialog accepts input within N ms of Cmd+P on a
  workspace of medd's own size* — taken the way the reading measure was, on the real thing.
  Until that number exists this bullet is a hope, and it belongs on increment 12's measurement
  list beside the large-document thresholds.

**The vacuity question for this one:** would the test still pass if the command were synchronous?
If yes, it is testing that the dialog exists, not that it opens promptly.

---

## 2. A symlink cycle makes the walk unbounded, silently

Nothing in §9 or the notes mentions symlinks, and `dir_list` never had to care because it descends
one level. A recursive walk does, and `fs::metadata()` follows links — the existing code relies on
that deliberately (`dir_list` shows a dangling symlink as inert *because* `metadata()` follows and
fails).

Demonstrated, not argued. A workspace containing `notes/loop -> ../` re-enumerates the same
document at every level and only terminates on an artificial depth cap:

```
files visited before bailing at depth 40: 20
hit the artificial depth cap: True          <- unbounded without it
```

The failure mode is the bad one: no crash, no error, no result. On a threadpool thread it spins
one worker forever while quick-open simply never populates — indistinguishable, from the user's
side, from a slow walk. **Criterion: a workspace containing a directory symlink to an ancestor
completes the walk and yields each document exactly once.** Cycle protection by resolved-path
identity, not by a depth cap, since a depth cap turns an infinite walk into a wrong one.

Worth checking the same fixture for the sibling case: a symlink pointing *outside* the root.
`workspace.rs` already resolves before comparing for `classify`; the walk needs the same rule, or
quick-open lists files the workspace does not contain.

---

## 3. The walk must not hold the workspace lock

This follows from the architect's own concurrency observation, one step further than they took it.

`commands.rs` holds `Mutex<Option<Workspace>>` across `ws.dir_list(&path)` today, which is safe
because that call is one level and returns in microseconds. A recursive walk of a real workspace is
not that. If the async command acquires the same mutex and holds it for the duration, **every sync
command blocks behind it** — and sync commands run on the main thread, so the UI freezes for the
length of the walk. That is precisely the outcome the async command was chosen to avoid, reachable
by holding a lock the old code could hold safely.

**Criterion: the walk copies the root out of the mutex and releases it before touching the
filesystem.** Testable directly — take the lock on another thread mid-walk and confirm it is
available.

---

## 4. Concurrency becomes tested, not merely assumed

The notes say `DocumentStore`'s single mutex "is defensive about this and holds, but it is the
first time that assumption gets tested rather than assumed." Nothing tests it yet, and *assumed
and holding* looks identical to *tested and holding* right up until it doesn't.

**Criterion: one test runs the walk concurrently with a `document_write` on the same workspace**
and asserts both complete with correct results. It is the first time two commands can genuinely
overlap; the value is in having run it once, not in the assertion being clever.

---

## 5. The fixture must contain what the filters exclude

A test asserting `node_modules` is excluded passes trivially against a fixture that has no
`node_modules`. The same for `target/`, dotfile directories, and non-Markdown files. This project
has already shipped a fixture whose comment claimed it exercised "every rendering feature v0.1
claims (R-1…R-7)" while containing no images at all — the one requirement with a history of silent
failure.

**Criterion: each exclusion has a fixture entry that would be found if the rule were removed**, and
the mutation harness carries a mutant per rule to prove it. `scripts/mutants.sh` is where those
belong; a rule with no mutant is a rule nothing is checking.

---

## 6. Cache invalidation, asked the way that can fail

"Invalidated by `tree:changed`" has an obvious test that proves nothing: populate, fire the event,
walk again, assert the result is current. That passes whether or not invalidation happened, because
a second walk returns current results either way.

**The test has to establish that a stale result would otherwise be served.** Populate the cache,
change the filesystem *without* firing the event, confirm the stale list comes back, then fire the
event and confirm it does not. Only the first half makes the second half mean anything.

**And the storm case.** `tree:changed` fires on every filesystem batch; a `cargo build` in a medd
workspace produces a long run of them — the architect's own note says `target/` is unignored today,
which is 45,805 files' worth. If invalidation triggers an eager re-walk, a build becomes a sequence
of full workspace walks. **Criterion: invalidation marks the cache stale; the walk happens on next
use.** Same distinction as the shutdown latch: what arrives during the work must not extend it.

---

## 7. Measure the result, since the problem was stated as a measurement

The notes open with 1,675 files enumerated per useful result. The fix should close that with a
number, not an assurance: **record the post-filter enumeration count and wall-clock walk duration
on medd's own repository**, next to the 41,875 it replaces. That is the same discipline as the
reading measure, and it is how anyone later knows whether a change to the ignore rules helped or
hurt.

---

## Not in scope for this pass

W-6's live sidebar updates, which stay deferred. The `App.svelte` error placement the notes flag
(a top-level `{#if error}` replacing the whole tab UI) is a real bug and already recorded against
increment 10 — worth fixing once, not twice, and not here.
