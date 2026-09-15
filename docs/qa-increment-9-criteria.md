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

### 3a. The async command makes two *existing* lock-across-syscall sites live

Sharper than I first put it. The hazard is not only that the new walk might hold the workspace
lock — it is that **two places already do, and nothing has been able to notice.** Every command
today is `ExecutionContext::Blocking`, so commands cannot overlap each other; a lock held across a
syscall blocks nobody because there is nobody to block. The async command is the first thing that
can contend, which makes both existing sites real on the day increment 9 lands, not later.

`document_read` (`commands.rs:111`):

```rust
if let Some(ws) = workspace.lock().unwrap().as_ref() {
    if let Ok(canonical) = path.canonicalize() {          // syscall
        let _ = app.asset_protocol_scope().allow_directory(dir, false);
        let _ = watcher.lock().unwrap().watch_non_recursive(dir);   // FSEvents registration
```

The guard is a temporary in the `if let` scrutinee, and on **edition 2021** those live for the
whole `if let` body. Verified rather than read off the reference — the same shape, compiled with
`--edition 2021`:

```
guard STILL HELD inside the if-let body  <- edition 2021
copy-then-use: guard released before the body   <- safe
```

(Edition 2024 changes this. `src-tauri/Cargo.toml` says `edition = "2021"`, so the guard is held.)

`workspace_open` (`commands.rs:63-71`) is the heavier one: it holds **both** the workspace and
watcher locks across `watch_recursive(ws.root())` — registering a recursive FSEvents watch over an
entire tree.

Lock *order* is consistent (workspace → watcher at both sites), so there is no AB/BA deadlock, and
that is worth keeping true rather than discovering later. The existing background contender,
`run_event_loop`, copies the root out and releases immediately, which is the pattern the walk
should follow and the reason the watcher thread has never made this visible.

**Criterion, generalised: no lock is held across a filesystem or OS call.** Copy what is needed out
and release first. Two existing sites need it; the walk must not add a third.

### 3b. Verifying the extraction: the signature has to take an *owned* root

The extraction only removes the possibility if the extracted function cannot be handed a guard.
The obvious check is that it takes a root rather than an `&Workspace` — but that is not sufficient,
and the insufficient version is the one Rust style will push a reviewer toward.

`&Path` **reads as extracted and achieves nothing.** `ws.root()` borrows out of the `Workspace`,
which borrows out of the `MutexGuard`, so the guard must stay alive for the whole call. Only an
owned `PathBuf` makes the guard impossible to hold, because it cannot outlive the statement that
produced it. Demonstrated:

```
&Path   : lock available during call? false     <- guard still held
PathBuf : lock available during call? true      <- guard released
```

`fn scope_and_watch(root: &Path, ...)` is the idiomatic signature, would pass review, and leaves
the invariant exactly as violated as before — with the added cost that it now looks addressed.
`fn scope_and_watch(root: PathBuf, ...)` — or taking `&Path` from a root the caller has already
copied out — is the one that holds.

**And check it first.** The availability test is only meaningful once the signature is right: a
test that takes the lock on another thread and finds it *available* while running against a
`&Path` signature has proved the test wrong, not the code right — it means the test never actually
overlapped the call. Ordering the checks signature-then-availability is not tidiness; the wrong
order spends the investigation on the test instead of the code, which is the same trap one level
out.

**Criterion: check the signature, not the call site.** A call site that currently copies first can
be edited back; a signature that only accepts owned data cannot be, without the change being
visible in the diff as a type change.

### 3c. The comment is a convention living in code

`conventions.md` records that an invariant belongs where it is enforced. The lock-order comment is
that rule applied to a comment: it goes at the `watcher.lock()` line in each of the two functions —
the point where the *second* lock is taken, which is where ordering begins to matter and where
someone adding a third is looking at a working example. **Criterion: it is at the acquisition, not
at the top of the file.** A file-header note is read once, by someone who is not yet adding a lock.

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
