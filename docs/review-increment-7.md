# Increment 7 — design review

**Reviewer:** principal architect
**Reviewed:** commit `45c9dd9` "Increment 7: autosave, filesystem watcher, and external-change handling"
**Against:** [architecture.md](architecture.md) §3, §4, §6; [decisions.md](decisions.md) D-5, D-11; [plan-v0.1.md](plan-v0.1.md) increment 7
**Status:** untracked. Written in the gap between increments per plan-v0.1.md's handoff convention. Accept, move, or delete as you see fit.

Every finding below was verified by running code, not by reading it. Probe harnesses are
described inline so each claim can be re-run. The design-change requests are routed to the
leader; nothing here has been changed in the tree.

---

## Summary

Increment 7 is well built. The compare-and-swap is correct, the full-duration lock is correct and
properly tested, own-write suppression by content hash is the right mechanism, and the frontend
state machine is small and legible in the way §3 asked for. The 41 Rust tests and the frontend
conflict-machine tests all pass, and they test the right things.

The holes are all of one shape, and it is the shape the symlink finding had: **§3 specifies the
document lifecycle as a sequence of discrete, settled states, and the implementation is a
concurrent system in which those states move underneath each other.** Six findings, in severity
order:

| # | Finding | Severity |
|---|---|---|
| 1 | A clean external reload **corrupts** a CRLF document and autosaves the corruption | Data loss. Blocker. |
| 2 | "Keep mine" leaves the document **unsaved indefinitely** when no debounce timer is pending | Data loss. Blocker. |
| 3 | A write settling after a resolution **clobbers** the resolved state; produces a banner nobody's edits caused | Trust. Fix before v0.1. |
| 4 | `detached` is a **terminal, invisible** state with no recovery and no UI | Data loss. Fix before v0.1. |
| 5 | Any read failure — not just `NotFound` — is reported as deletion | Correctness. Fix before v0.1. |
| 6 | An external change to non-UTF-8 content is lossily converted, and the CAS will let medd write the lossy version back | Data loss, narrow. Fix before v0.1. |

Plus one latent issue (own writes emit `tree:changed`, §7 below) and a drift ledger (§8).

---

## 1. A clean external reload corrupts a CRLF document, then autosaves the corruption

**Blocker.** This is the worst finding, and it lands on the D-11 path whose entire justification
is that it is the safe, silent, common one.

§1 says "the frontend owns the document. The text of an open document lives in the WebView,
because that is where the editor component runs." What that means on this platform, and what §3
never says, is that **CodeMirror normalises line breaks.** `EditorState.create({ doc })` splits on
`\r\n`, `\r`, and `\n`, and `doc.toString()` rejoins with `state.lineBreak`, which defaults to
`\n`. So for a CRLF document there are immediately two different texts in play:

- `tab.currentText` / `tab.lastSyncedText` — the raw CRLF bytes, as `document_read` returned them
- the retained `EditorState` — the same document, LF-normalised

`applyExternalContent` then computes its minimal diff **across that boundary**:

```ts
const change = computeMinimalChange(state.doc.toString(), newContent)
//                                  ^ LF-normalised    ^ raw CRLF from disk
```

Every offset after the first line break is wrong, and the resulting "minimal" change is neither.
Verified end-to-end through the real production modules (`openTab` → real `EditorView` →
`document:changed-on-disk` listener → autosave debounce):

```
open "a\r\nb\r\nc"          disk changes to "a\r\nB\r\nc"   (one letter, b -> B)

computeMinimalChange("a\nb\nc", "a\r\nB\r\nc")
  -> { from: 1, to: 3, insert: "\r\nB\r" }        <- note the trailing bare \r

buffer after applying     : "a\nB\n\nc"           <- a blank line that exists in neither version
disk, as LF               : "a\nB\nc"
match?                    : NO
```

The trailing bare `\r` in the computed insert is itself a line break to CodeMirror, which is where
the spurious blank line comes from.

It does not stop there. Because `lastSyncedText` is set to the CRLF text while the buffer holds LF
text, `dirty` is *true* immediately after a reload that was supposed to leave the document clean
— and the `dispatch` through the mounted view fires the update listener, which schedules an
autosave. One second later:

```
document_write { path: "/w/a.md", content: "a\nB\n\nc", expectedHash: "h2" }
```

The `expectedHash` is correct, so the compare-and-swap **passes**, and medd writes content that
matches neither the user's version nor the external one over the user's file. The user typed
nothing. The file is corrupt and the banner never appeared, because the buffer was clean and D-11
says the clean case is silent.

A second, milder consequence of the same root cause: **the first keystroke in any CRLF document
rewrites every line ending in the file.** Verified — open `"a\r\nb\r\nc"`, type `X`, autosave
writes `"a\nb\ncX"`. That one is defensible as a convention (many editors do it) but N-7 says
files stay plain and portable, and nothing in the docs has decided it.

`docs/` is silent on line endings from end to end. This is not a corner: `.md` files arrive with
CRLF from Windows collaborators, from `* text=auto` checkouts, and from Confluence exports —
which is the content this product is aimed at.

**Design decision needed (routed to the leader).** Three options; I recommend the first.

1. **Normalise on read, restore on write.** `document_read` records the document's dominant line
   ending alongside its content and hands the frontend LF-only text; `document_write` converts
   back before hashing and writing. One representation in the frontend, the file's own convention
   preserved on disk, and `computeMinimalChange` compares like with like. Costs a field on the
   read/write contract and a per-document `lineEnding` on the tab.
2. **Normalise on read and write LF.** Simplest, and honest if stated: medd converts CRLF
   documents to LF on first save. Cheaper than (1) by one field, but it silently rewrites a whole
   file the user may only have meant to read, which is hard to square with N-7.
3. **Tell CodeMirror not to normalise** (`EditorState.lineSeparator`). Rejected: it makes the
   separator a per-document facet, which means it is baked into the retained `EditorState` at
   `openTab` time and cannot follow an external change that alters the convention. It also leaves
   mixed-ending documents with no correct answer.

Whichever is chosen, §3 needs a paragraph saying what "the frontend owns the document" means for
a component that normalises what it holds, and the invariant it establishes — *`lastSyncedText`
and the retained `EditorState` must always be in the same line-ending convention* — needs stating
where someone will read it.

---

## 2. "Keep mine" leaves the document unsaved indefinitely when no timer is pending

**Blocker**, and it is the answer to the question about §3's "next keystroke" wording — it
inverts that question.

The pending-timer behaviour dev found is real and intended (ruling in §9 below). But it is the
*lucky* case. `resolveConflictKeepMine` clears the conflict and adopts the new baseline; it does
not schedule anything. So whether the user's version ever reaches disk depends entirely on
whether a debounce timer happened to survive the conflict. Often it does not:

**Route A — a rejected compare-and-swap write.** This is the plan's own named case ("a rejected
CAS write becoming a conflict"). The timer fired to *perform* that write, so it is spent by the
time the conflict exists:

```
type X              -> timer scheduled
t = 1s              -> timer fires, document_write, rejected with Conflict
                       markConflict; timer is gone
click "Keep mine"   -> conflict cleared, baseline adopted, dirty = true
t = 61s             -> writes in the last 60 seconds: 0
                       still dirty, still unsaved
```

**Route B — a watcher event arriving after the debounce already fired.** Same outcome, verified:
the write goes in flight, the external change arrives while it is un-acked so the tab reads as
dirty, `markConflict` fires, and there is no timer left. Zero writes in the following 60 seconds.

So for both routes: the user clicks the button that means "my version wins", and their version
sits in memory, unwritten, with no banner, no dirty indicator, and nothing on screen to suggest
anything is pending. Close the tab — which P-2 promises is always safe — and the edits are gone.

This is precisely the failure D-5 chose autosave to eliminate ("there is no unsaved-changes state
to manage"). Increment 7 reintroduced one, in the single place in the product where the user has
just been told their work is at stake.

**Fix.** `resolveConflictKeepMine` (or `keepMine` in `doc/`) must schedule an autosave on
resolution. That keeps D-11's "does not write immediately" intact — it is still the debounce that
writes, still through the one compare-and-swapped path, still no second write path — while making
"resumes normal autosave" mean what it says. `reload()` does not need this: it ends clean by
construction.

This also removes the need to defend the pending-timer coincidence at all. Once resolution always
schedules a tick, the behaviour is the same whether a timer survived or not, and §3 can describe
one thing instead of two.

---

## 3. A write settling after a resolution clobbers the resolved state

`performAutosave` captures `path` and `textToWrite`, awaits `document_write`, and then calls
`markSynced` or `markConflict` — **without checking whether the tab's state moved while the await
was pending.** It can, and §3's flowchart has no notion of a write being in flight when one of
its transitions fires.

**3a — Reload during an in-flight write.** Verified:

```
type X                  -> autosave in flight, expectedHash h1
external change arrives -> tab dirty -> markConflict
user clicks Reload      -> currentText "DISK", lastSynced "DISK", hash hDISK, conflict null
                           (correctly resolved, buffer clean)
the stale write settles  -> markSynced(path, "v1X", "hSTALE")
                           currentText "DISK", lastSynced "v1X", hash hSTALE
                           dirty = TRUE
```

The resolved tab is now dirty against a baseline that describes neither the buffer nor what the
user asked for, and `expectedHash` points at content the user explicitly discarded. If the stale
write instead *fails* with `Conflict`, `markConflict` runs and **the banner reappears on a clean
buffer the user already resolved** — offering Reload/Keep mine for a conflict that no longer
exists.

**3b — Close and reopen during an in-flight write.** Worse, because it fabricates a conflict from
nothing. Verified:

```
type X in a.md          -> autosave in flight
close the tab
reopen a.md             -> fresh read: currentText "FRESH FROM DISK", hash hFRESH
the stale write settles  -> markSynced clobbers it: lastSynced "v1X", hash hSTALE
                           tab is dirty, on a file the user has not touched since opening
next autosave           -> writes with a stale hash -> CAS rejects -> conflict banner
```

A conflict banner on a document nobody edited is exactly the failure D-11's rationale names as
fatal to the whole design: *"A banner that appears after edits nobody made is how users learn to
dismiss it unread."* The read-lock fix in increment 2 was made for this reason on the Rust side;
the same class of bug is now live on the frontend side.

**Fix.** A per-tab generation counter, incremented by every state-machine transition
(`markConflict`, `applyExternalContent`, `resolveConflictKeepMine`, `markDetached`, `closeTab`).
`performAutosave` captures it before the await and drops its result if it has moved. This is the
frontend's equivalent of "the mutex covers the whole of the read" — §3 should state the invariant
in the same terms: *a write's outcome may only be applied to the state that issued it.*

---

## 4. `detached` is terminal and invisible

§3 says: *"The tab stays open holding its text, marked as detached from disk; autosave for it is
suspended until the user saves it somewhere."* Two things are wrong with that sentence as v0.1
reality.

**There is no "somewhere".** v0.1 has no Save As, no Save, no relocation affordance of any kind.
So "suspended until the user saves it somewhere" describes a state with no exit.

**There is no "marked".** `ConflictBanner` renders on `tab.conflict`. Nothing anywhere renders on
`tab.detached` — I checked `App.svelte`, `TabBar.svelte`, and the banner. The user's file is
deleted, autosave silently stops forever, and they keep typing into a tab that will never be
written, with nothing on screen saying so.

**And it does not recover if the file comes back.** Verified in Rust: `check_external_change`'s
failure branch does `last_known.remove(&key)`, so the document stops being tracked. Recreate the
file and the watcher sees an *untracked* path — it folds into `tree:changed`, and no
`document:changed-on-disk` is ever emitted. The tab is detached permanently:

```
after delete    : Some(Removed)
after recreate  : None          <- tab stays detached forever
is_tracked      : false
```

Delete-then-recreate is not hypothetical — `git checkout` across branches does it, as do several
editors' save strategies.

One piece of good news, verified: **Neovim's write-by-rename is classified correctly** as
`Changed`, not `Removed`, because the ~100ms coalescing window collapses the unlink and the
rename before `check_external_change` re-reads. That is the design working as intended and is
worth recording in §6 as a verified property rather than leaving it to be rediscovered.

**Fix, smallest version that is honest for v0.1.** (a) Render a banner for `detached` — the tab is
already styled for one, so this is the existing component with different copy. (b) Clear
`detached` when a `document:changed-on-disk` arrives for that path, and keep the path tracked on
deletion (or re-track it) so that event can happen. (c) Amend §3 to stop promising a save-elsewhere
affordance that v0.1 does not have, and say instead what v0.1 actually does.

---

## 5. Any read failure is reported as deletion

§3 says "**File is gone** → emits `document:removed-on-disk`". The implementation:

```rust
match fs::read(&key) {
    Ok(bytes) => { ... }
    Err(_) => {                      // <- any error at all
        last_known.remove(&key);
        Some(ExternalChange::Removed)
    }
}
```

Verified with the file still very much present:

```
chmod 000, file exists  -> Some(Removed)
still tracked?          -> false
```

`EACCES` from a permission change, `EIO`, or `EMFILE`/`ENFILE` under descriptor pressure — the
last of which is most likely during exactly the kind of filesystem storm (`git checkout`, `npm
install`) that generates watcher traffic in the first place — all latch the tab into the terminal
state described in §4. **Fix:** only `ErrorKind::NotFound` means removed; anything else should be
left tracked and treated as "could not determine", which given own-write suppression's design is
safe to ignore for one event.

---

## 6. Non-UTF-8 external change is converted lossily, and the CAS will let medd write it back

`read()` refuses non-UTF-8 with `MeddError::NotUtf8`. `check_external_change` does not — it uses
`String::from_utf8_lossy`. §3 does not mention the asymmetry, and it is load-bearing. Verified:

```
tracked file "hello" is externally overwritten with bytes 68 69 ff fe

check_external_change -> Changed {
    content: "hi\u{FFFD}\u{FFFD}",                       <- lossy, pushed to the frontend
    hash:    <blake3 of the RAW bytes>                   <- NOT the hash of that string
}
```

The hash the frontend adopts as `expectedHash` is the hash of what is genuinely on disk. So a
subsequent autosave's compare-and-swap **passes**, and medd writes the replacement-character
version over the file's real bytes. Narrow — it needs the file to become binary externally while
open — but it is silent destruction of content medd explicitly refuses to open in the first place.

**Fix.** Treat a non-UTF-8 external change the way `read()` treats a non-UTF-8 open: do not push
the content. Either emit a dedicated event and detach the tab, or emit nothing and leave the
document tracked with its old hash so the next autosave is rejected as a conflict rather than
accepted. Either way §3 should say which, because right now it says nothing.

---

## 7. Own writes emit `tree:changed` — latent, but it is the same gap as the symlink one

Own-write suppression is specified entirely in terms of the document's content hash. But an
atomic write is **not a single-path event**. FSEvents reports three paths for one autosave, and I
ran it to be sure:

```
reported paths for one document_write:
  /root                       -> untracked -> sets tree_changed
  /root/note.md               -> tracked, hash matches -> suppressed  (correct)
  /root/.medd-note.md.tmp     -> untracked -> sets tree_changed
  /root/note.md               -> suppressed

tree_changed after one autosave = true
```

`is_ignored_in_workspace` cannot catch the temp file: it deliberately pops the changed entry's own
name before testing ancestors, which is right for `.env` and wrong here. And the containing
directory surfaces as its own untracked path regardless.

Harmless today because `tree:changed` is not wired to the sidebar. In v0.3 when W-6 lands, **every
autosave debounce will re-list every expanded directory in the tree.** Worth fixing now, while the
reason is in view.

Two things to note for the leader and for QA:

- §6 has no concept of medd's own temp files as watcher noise, nor of an atomic write being a
  multi-path event. That is the §3/§6 gap to record.
- The existing test `own_write_produces_no_notification` **cannot catch this.** It asserts
  `check_external_change(path).is_none()` for each reported path — which is true for the temp file
  (it is untracked) — and never exercises the `tree_changed` decision. The test passes while the
  behaviour it is named for is violated. §12's claim that "own-write suppression" is covered is
  therefore overstated; the assertion needs to be about emitted events, not about
  `check_external_change` in isolation.

---

## 8. Drift ledger — where architecture.md no longer describes what exists

Separating **drift to correct** (code should change, or docs should) from **discovery to record**
(the implementation is right and the design did not know this yet).

### Drift to correct

**`document_close` is specified in §4 and does not exist.** "Stop tracking; drop a loose-file
watch." It is not implemented, not registered in `main.rs`, and `closeTab` does not call it. It is
increment 7's own concern and it was missed. Consequences, all real: `last_known` grows for the
whole session (§8's "closed tabs free everything" is true on the frontend and false in Rust); a
loose document's directory watch is never released; its asset-protocol grant is never revoked,
contradicting §11's "scoped to the workspace root and the directories of **open** loose
documents"; and the closed path stays tracked, so `is_tracked` keeps suppressing `tree:changed`
for it.

**A loose document opened with no workspace open gets no watch and no asset scope.** In
`document_read` the whole block is inside `if let Some(ws) = workspace...`. P-3 is a Must, so a
document with no external-change detection at all is a requirement gap, not a nicety. Barely
reachable in v0.1 (the tree is the only opener), fully reachable once increment 10 routes
`medd ~/somewhere/loose.md` into a fresh instance.

**§3's rejected-write path does not say what happens to `last_known`.** It stays at the stale
value. I traced every interleaving I could construct and it is self-healing, because
`check_external_change` always re-reads current disk state rather than trusting a queue of
hashes — that is a genuine strength of the design. But it is load-bearing and undocumented, which
is how the symlink gap started.

**§6 describes a filter that does not exist.** "Paths not under a watched scope are dropped."
There is no scope check; `check_external_change` runs for every reported path. Harmless, since
notify only reports watched paths — but a doc describing a defence that is not there is worse than
one that does not claim it.

**README's status section is eight increments stale** — "Nothing else works yet — no file reading,
no editor, no preview." It is the public front page of the repo.

**Code comments reference an "increment-3 report" that does not exist as a document**
(`workspace.rs`). Those decisions live only in commit messages.

### Discovery to record

**The conflict state machine lives in `tabs/`, not `doc/`.** §2's module layout assigns "dirty
tracking, autosave debounce, conflict state machine" to `src/doc/`. In reality `doc/` holds the
debounce, the IPC calls and the listeners, while the state itself — `conflict`, `detached`,
`lastSyncedText`, `expectedHash`, `markConflict`, `applyExternalContent`,
`resolveConflictKeepMine` — lives in `tabs/tabs.svelte.ts`. That is the **right** call: the state
is per-document and keyed by tab identity, so it belongs with the thing that owns tab identity.
But it is a real divergence, and it is the reason `doc.ts` needs the `setOnDocChanged`
registration hook to dodge a circular import — machinery §2 never anticipated. Record it; don't
"correct" it.

**`expectedHash` has two owners.** The frontend holds it per tab; Rust holds `last_known` per
path. Two sources of truth for one fact. That is defensible (Rust's copy suppresses its own echo,
the frontend's is the CAS baseline it will send) but §3 presents it as one value the frontend
"adopts", and the seam between the two copies is precisely where findings 2 and 3 live.

**Neovim's write-by-rename classifies correctly as `Changed`.** Verified. It depends on the
~100ms coalescing window collapsing the unlink and the rename, which it comfortably does. Worth
recording in §6 as a verified property, with the dependency named — it is the kind of thing a
future change to `COALESCE_WINDOW` could break invisibly.

**Full-duration locking is confirmed adequate, and cheap.** The single store-wide mutex now also
covers `check_external_change`, which it must. `document.rs`'s own comment already flags the
serialisation cost and when to revisit it. No change needed; worth promoting into §3 so the
decision is visible outside the source file.

---

## 9. Ruling: the "Keep mine" pending-timer behaviour

**Intended. dev's reading is correct.** But the question as posed understates the problem, and the
fix is not only wording — see finding 2.

On intent: "Keep mine" means the user's version wins. Having it land about a second later, on the
tick already in flight, is what they asked for. The competing reading — "Keep mine" means "stop
touching my file until I act" — cannot be made consistent with D-11 without changing D-11's
semantics, because §3 already promises that the *next keystroke* commits it. A reading under which
the timer firing is wrong makes the keystroke wrong too. So that reading is not available, and
this is not a decision that needs to go back to the locked decision.

The real reason "does not write immediately" is right is not the one §3 gives. §3 states a
mechanism and lets it stand as the rationale, which is why the text broke as soon as the mechanism
turned out to have two paths. The actual reason is **one write path**:
`resolveConflictKeepMine` stays a pure state transition with no I/O, so there is exactly one place
in medd that writes a document, and it is the place that is compare-and-swapped and tested. That
is worth protecting; "the next keystroke commits it" is not, and never was the point.

Once finding 2 is fixed — resolution schedules a tick — the pending-timer coincidence stops
mattering, because the behaviour is identical whether a timer survived or not. §3 then has one
thing to describe. Proposed replacement for §3's paragraph, to be applied **after** finding 2:

> **"Keep mine" resumes autosave rather than performing a write of its own.** It adopts the new
> disk hash as the compare-and-swap baseline, clears the conflict, leaves the buffer untouched,
> and schedules an autosave tick. Because `dirty` is derived (`currentText !== lastSyncedText`)
> and `lastSyncedText` becomes the *disk's* content, the buffer is dirty against the new baseline,
> so that tick writes the user's version over disk — about a second after the click, with no
> further keystroke needed.
>
> What "does not write immediately" buys is a single write path: `resolveConflictKeepMine` is a
> pure state transition with no I/O, so there is exactly one place in medd that writes a document,
> and it is the one that is compare-and-swapped and tested. Scheduling a tick rather than relying
> on whichever timer happens to have survived the conflict is what makes that true in every case
> rather than most of them.
>
> The consequence to be honest about: **"Keep mine" discards the disk version within about a
> second, and the user has never seen it.** While `Diff…` is deferred (D-11's stated cost), the
> banner's wording is carrying that weight on its own.

That last paragraph is the other thing I want on the record. **"Keep mine" is the only button in
medd whose effect is to destroy someone else's version of a document, and it is labelled as
though it were the passive option.** "Keep mine" does not say "and discard theirs." D-11 correctly
defers `Diff…`, but with it deferred the label is the entire safeguard. Changing the copy is a
product call, not mine — routed to the leader, no recommendation forced, but I would not ship
v0.1 without looking at it.

---

## 10. What I would gate increment 7 on

Blockers: findings **1** and **2**. Both are silent data loss on paths the design describes as
safe, and both are cheap to fix now and expensive to fix after quick-open, the CLI and session
restore are built on top.

Fix before v0.1, not necessarily before increment 8: **3**, **4**, **5**, **6**, and the missing
`document_close`.

Record now, fix before v0.3: **7**.

Tests worth adding alongside the fixes, since §12's standard is that anything which can silently
lose data is tested automatically:

- A CRLF document through a clean external reload — buffer matches disk, no autosave triggered.
- "Keep mine" via a rejected CAS write — a write lands within the debounce window.
- An in-flight write settling after Reload, after Keep mine, and after close-and-reopen — the
  resolved state survives in all three.
- Delete then recreate — the tab recovers.
- `chmod 000` — not reported as removed.
- A non-UTF-8 external change — the lossy content never reaches the frontend.
- One autosave emits no `tree:changed`. Asserted on emitted events, not on
  `check_external_change` in isolation — see §7.
