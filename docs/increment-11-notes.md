# Increment 11 — what the three bullets don't say

**From:** principal architect
**For:** [plan-v0.1.md](plan-v0.1.md) §11, [architecture.md](architecture.md) §7
**Status:** written before implementation. Every claim read from the code at `4c0ed9a`.

Eight items. The first two mean increment 11 cannot be built as specified — not because the design
is wrong, but because the primitive it depends on cannot do what §7 says it does.

---

## 1. `atomic_write` cannot write a file that does not exist, so the first state save fails

§7: *"Both are written through the same atomic write path as documents (§3)."*

```rust
fn atomic_write(target: &Path, content: &[u8]) -> Result<(), MeddError> {
    …
    let perms = fs::metadata(target)      // <- the target must already exist
        .map_err(|e| MeddError::io(target, e))?
        .permissions();
```

Its own doc comment says so: *"`target` must already exist (compare-and-swap always re-reads it
first)."* That precondition is free for documents — a CAS write has just read the file — and
**false for both state files on first run**, when neither exists.

So the very first `state.json` write returns `Io { NotFound }`. And one layer out, the same is true
of the directory: Tauri's path API *resolves* `~/Library/Application Support/medd/`, it does not
create it, so on a fresh machine even the temp file has nowhere to go.

Both are trivially fixable and neither is discoverable from §7, which is the point of saying it
now: **the first thing increment 11 does fails twice, on a fresh install only** — the one
configuration the developer building it is least likely to be in.

## 2. `atomic_write` is private, and its only caller is compare-and-swapped

`fn atomic_write` — no `pub`. One caller: `DocumentStore::write`, which requires an
`expected_hash`. A hash is meaningless for `state.json`, so **there is no reachable way for
`state.rs` to perform an atomic write today.**

Increment 11 therefore has to expose it or copy it. Copying would be a second implementation of
the most dangerous primitive in the product, in the module least able to justify it — the ninth
instance of the pattern this project keeps paying for, in the worst possible place.

But exposing it as-is costs something real. Right now *"no document write bypasses the
compare-and-swap"* holds **structurally**: `atomic_write` is private and the only thing that can
reach it CASes first. Make it `pub` and that becomes a convention again — the exact downgrade we
just spent three changes reversing elsewhere.

**Proposed shape: move the atomic-write protocol into its own module** — `atomic.rs`, with
`pub(crate) fn write(target, content)` — and have `document.rs` and `state.rs` both call it. Then:

- one owner, no copy;
- the CAS stays in `DocumentStore::write`, which remains the *only* function that writes a
  document, so the structural property survives;
- the staging-file naming (`TEMP_PREFIX`, `temp_file_name`, `target_of_temp`, `is_staging_file`)
  moves with it, because those are properties of the atomic-write protocol rather than of
  documents — which is where they arguably always belonged, and it is why the watcher currently
  imports a staging-file predicate from `document.rs`.

Two decisions fall out and need making rather than defaulting: whether `write` creates an absent
target (yes, per §1), and what permissions a newly created file gets, since there is no target to
copy from. The temp file's own default mode is the right answer for both state files; it should be
stated rather than inherited by accident.

## 3. The sweep will never clean state-file litter — exactly as its doc comment predicted

`sweep_abandoned_temps` requires the staging file's target to be Markdown. Simulated against the
real predicates:

```
.medd-state.json.tmp      target='state.json'      swept=False
.medd-settings.json.tmp   target='settings.json'   swept=False
.medd-note.md.tmp         target='note.md'         swept=True
```

So an interrupted state write leaves litter in Application Support permanently. Low harm — the
directory is invisible and not under git, which is the whole reason the workspace case mattered —
but worth recording for two reasons.

First, this is the `.md` guard's documented failure mode arriving one increment later, in the exact
words it was written in: *"if medd ever gains the ability to edit other file types, this guard
silently stops sweeping their staging files."* It fails safe, as designed. The prediction was
right, which is a better outcome than the guard being wrong.

Second, it needs a decision rather than a discovery: either the state directory gets its own sweep
with its own guard, or the litter is accepted and said so in §7. I would accept it — one stale
temp file in an invisible directory per interrupted write, with no git to show it, is not worth a
second sweep path — but it should be a line in §7, not a silence.

## 4. Nothing validates a recents entry, and staleness is far likelier than corruption

§7 and §11 both handle a state file that **fails to parse**. Neither mentions one that parses
perfectly and contains paths that are gone.

That is the common case, not the rare one: a project renamed, a clone deleted, an external drive
unmounted, a worktree removed. Corruption needs a crash mid-write or a bad edit; staleness needs
only time.

Two consequences, neither specified:

- **Bare `medd` restoring a dead last workspace.** §11 says it "restores the last workspace" with
  no failure path. `Workspace::open` returns `Err` for a missing directory, and nothing says what
  happens next.
- **The welcome pane lists ten recents, any of which may be dead.** Clicking one surfaces an error
  where a workspace was expected.

**Recommended: validate on read.** Drop recents whose directory no longer exists, and treat a dead
last workspace as *no workspace* — which lands on the welcome pane, which is exactly the right
place to be. One filter, and it makes the welcome pane's list honest rather than optimistic.

Worth noting what is *not* broken here: `App.svelte`'s top-level `{#if error}` branch replaces the
content area but sits inside `<main>`, so the header keeps its *Open Folder…* button. A failed
restore is recoverable. (That error placement is still the separate fix already noted against
increment 10's `medd <nonexistent-file>` case.)

## 5. §7's no-clobber guarantee is currently true by accident

> *"A human may edit it by hand and medd will not clobber it."*

True in v0.1 — because **nothing writes `settings.json` at all.** There is no preferences UI (P-5
is a *Could*), so the only writer is a hand edit.

The moment a preferences UI exists, changing one setting in-app writes medd's whole in-memory
`Settings`, silently discarding any hand edit made since load. That is the lost-update problem
D-11 solved for documents, unsolved here and unmentioned.

Not a v0.1 defect. But the guarantee should say *why* it holds, because the sentence as written
reads as a property of the design rather than of the current feature set, and whoever adds
preferences will reasonably believe it is already handled. The honest form: settings are read once
and never written in v0.1; a preferences UI must re-read before writing.

## 6. The corruption path's own failure must not be fatal

> *"A file that fails to parse is renamed to `.bak` alongside, a fresh default is used, and the app
> launches."*

The rename can fail — a read-only directory, a permissions problem, a `.bak` that cannot be
replaced. Written naturally, `fs::rename(…)?` propagates and **kills startup**, which is precisely
the failure this rule exists to prevent. The rule needs its second half stated: *defaults are used
regardless of whether the backup succeeded.*

Also worth one line: a second corruption overwrites the first `.bak`. Acceptable — the more recent
failure is the more useful one — but say so, or someone will treat the first as preserved.

## 7. Semantic validity is not corruption, and the cap belongs on read

A `state.json` that parses but holds ten thousand recents, or a `lastWorkspace` pointing at a file
rather than a directory, takes the **success** path — the corruption handling never fires.

§7 says recents are *"capped at 10"* without saying where. If the cap is only applied on write, a
hand-edited or older-version file can carry any number, and the welcome pane renders all of them.
Enforce on read as well: it is one `truncate` and it makes the cap a property of the data rather
than of the code that last wrote it.

## 8. The plan carries a superseded ruling for the temp sweep

§11 is not the only place this lands, but it is adjacent enough to matter. The carried-fixes table
still says:

> Sweep in `workspace_open`, not the exit path… **Only sweep files older than about a minute**, so
> the documented two-instance race cannot have one instance delete another's in-flight temp file.

That ruling was replaced: the age heuristic was rejected outright, and the sweep runs from
`document_write` guarded by an advisory lock, which answers the two-instance question exactly
rather than probabilistically. The landed code and the plan now disagree.

This is the same shape as the correction that half-landed before — the conceptual documents were
updated and the operational one was left *looking* updated. Worth fixing in the same pass, since
anyone implementing from the table would build the design we rejected.

---

## Summary

| # | Item | Class |
|---|---|---|
| 1 | `atomic_write` requires an existing target; both state files are absent on first run, as is the directory | **Blocks the increment** |
| 2 | `atomic_write` is private with no hash-free caller — expose via a new `atomic.rs` owner, don't copy, don't just `pub` it | **Blocks the increment** |
| 3 | State-file litter is never swept, by the `.md` guard's documented design — accept it in writing | Decision |
| 4 | Nothing validates recents or the last workspace; staleness is likelier than corruption | Design gap |
| 5 | §7's no-clobber guarantee holds only because nothing writes `settings.json` | Record the reason |
| 6 | The `.bak` rename can fail; defaults must be used regardless | Design gap |
| 7 | A parseable-but-wrong state file bypasses corruption handling; cap on read | Design gap |
| 8 | The plan still carries the superseded age-based sweep ruling | Stale doc |
