# Design review — the two field bugs

**From:** principal architect
**Reviewing:** the leader's diagnoses and proposals, before authoring
**Checked against:** the code and docs at `9b8f74b`

Both diagnoses are correct. **One proposal has a latent bug, and one incident is not the kind of
thing it is being called** — and that second point changes what needs recording rather than what
needs writing.

---

## Incident 1 — the unwatched document

The diagnosis is right, and the two-arm harness with the control is the right instrument:
`assert!(events.is_empty())` passing against a broken harness is exactly the failure that needs a
positive arm beside it. The observation that `document_read` took a concrete `AppHandle` and was
therefore *untestable by construction* is the more useful half — that is why the path had no
coverage, not an oversight in test-writing.

### The proposal as stated introduces a worse bug than it fixes

> watch the document's own directory whenever a document is opened, not only when a workspace
> classifies it Loose

For a **root-relative** document that adds a non-recursive watch on a directory already inside the
recursive workspace watch. Wasteful, and mostly harmless — until `document_close` lands, which is a
queued carried fix whose entire job is to *release* those watches.

Then: a document directly in the workspace root has parent directory == the workspace root. So
closing that tab calls `unwatch(root)` — **the same path the workspace's recursive watch is
registered under.** That removes external-change detection for the entire workspace, silently, on
closing an ordinary tab. P-3 is a Must, and the failure is invisible: the tree stops updating and
no document reports changes, with nothing on screen and no error.

**The rule must be *watch unless already covered*, not *always watch*:**

```
if a workspace is open and the path classifies RootRelative  ->  already covered, do nothing
otherwise                                                    ->  watch the document's own directory
```

`is_loose` then genuinely answers *"which directory"*, and the no-workspace case falls into the
second branch because there is no coverage to inherit — which is the bug being fixed, arrived at
without creating the collision. Note this is the same `Option`-shaped trap in the other direction:
the fix must not treat *"no workspace"* and *"workspace, document inside it"* as one case either.

### The asset scope should be split, and the reason is not the one I expected

Your instinct is right; the argument I would have given is wrong, and the real one is sharper.

I expected the grant conditions to differ. They do not — enumerated, watching and scoping are
needed in exactly the same three cases (root-relative: neither; loose with a workspace: both; no
workspace: both). So *"they answer different questions"* is true but does not by itself justify
splitting, because today the answers coincide.

**What differs is revocation, and revocation is where it bites.** `document_close` must release
both, and the conditions are not the same:

- A **watch** may be released when no open document needs that directory. Releasing it wrongly
  costs P-3 — a Must — and fails silently.
- An **asset scope** may be released on the same condition, but releasing it wrongly costs R-4, a
  *Should*: images stop rendering, visibly. (**Superseded by the second amendment**: an asset scope
  cannot be released at all. The split survives; this reason for it was too kind to the API.)

Welded on the grant side, they will be assumed welded on the revoke side, and the first person to
get the shared release condition slightly wrong takes out a Must while believing they were
adjusting image loading.

And the argument that settles it regardless: **granting asset-protocol scope widens a security
boundary.** Increment 5 scoped the asset protocol deliberately, and the CSP is the stated
mitigation for `document_read`'s arbitrary-read surface — which the increment-12 audit has since
made a release gate. A widening of that boundary must not be a side effect of a function named for
watching. Two calls, two names, two reasons.

---

## Incident 2 — this is not a dead wire, and the distinction is the whole finding

The mechanism is exactly as described: the backend emits, nothing listens, `Tree.svelte` calls
`load()` once at creation. But:

> Both ends correct, the connection between them absent — the same class as the error wire-format
> bug.

**It is not that class.** The wire-format bug was an *accident*: both sides believed they were
connected and the names silently disagreed. This connection was never made, **on purpose, and
architecture.md says so in as many words** (§6):

> Live tree updates are a v0.3 item (W-6). The watcher exists in v0.1 anyway, because
> external-change detection (P-3) is a Must and needs it; `tree:changed` is simply not yet wired
> to the sidebar.

So this is **W-6 arriving early at the CEO's request**, not a defect. That is a perfectly good
reason to do it — they hit it, and W-6 is a stated *Should* rather than something new — but it is a
**scope change and has to be recorded as one.** Filed as a bug fix, the plan goes on saying v0.3
while the code says v0.1, and the next person reading §6 finds the sentence above describing
something that is no longer true. That is precisely the drift class this project has spent weeks
removing, and it would be self-inflicted.

Two doc edits, not a decision: amend §6's "not yet wired" sentence, and move W-6 in
`scope-mvp.md`/the plan from v0.3 with the CEO's request as the reason.

### Declining the button is right, and the case is stronger than the one made

Agreed, and on the CEO's own grounds. One fact makes it stronger than the complexity argument:

**A manual refresh already exists for subdirectories.** `Tree.svelte:53` renders the child
`<Tree>` conditionally on expansion, so collapsing a directory destroys the component and
re-expanding creates a new one — which calls `load()`. Collapse-and-re-expand *is* a refresh, today,
unlabelled and free.

The gap is only at the **root**, whose `<Tree>` is created once by `App.svelte` and never
destroyed. And the root is exactly where the CEO hit it: *"created a file in an open folder."*

So a refresh button would add a control that duplicates an existing capability everywhere except
one level. That is a better argument than "a button is more complexity", because it does not
require the CEO to accept a judgement about complexity — it points at something already true.

**One risk to state rather than discover.** Connecting the listener removes the *motivation* for a
manual override while the watcher can still legitimately miss things: the ignore rules, a path
outside the root, FSEvents dropping under load. For subdirectories, collapse-and-re-expand remains
the recovery. For the root there is none. Acceptable for v0.1 — the tree is not load-bearing for
data safety — but it should be a sentence somewhere rather than an assumption.

### Q1 — per-instance, and the architecture already ruled it

Not a judgement call. §6 already specifies the behaviour:

> a `git checkout` touching two thousand files produces one message, not two thousand … the
> frontend **re-lists only the directories it currently has expanded**

Per-instance is the implementation of that sentence. An App-level listener that re-keys the root
would re-list everything and then need expansion state lifted into a store to survive — which
duplicates state the component tree already holds correctly, and is strictly more complexity for a
coarser result.

**And the N-calls worry is smaller than it looks.** N is the number of *expanded* directories — a
handful, because a user expands a handful — and `tree:changed` is already coalesced upstream to one
event per debounced batch. So a two-thousand-file `git checkout` costs N `dir_list` calls in total,
not 2000 × N. Each is one level, bounded, and already on an async command.

### Q2 — no payload. Keep it `()`

Three reasons, in order of weight:

1. **A coalesced batch touches many directories, so one path cannot represent it.** The payload
   would have to be `Vec<PathBuf>`, and then every `Tree` instance filters it against its own
   path — more machinery in both runtimes than the N calls it saves.
2. **It would un-fold the thing `decide` is batch-shaped to fold.** `tree_changed` is deliberately
   a single boolean across the batch; that is why `decide` takes a batch and returns a batch, and
   why `drop_redundant_ancestors` can exist at all. Carrying paths reintroduces the per-file
   granularity §6 explicitly collapses.
3. **The saving is small and the cost is on a tested boundary.** N is a handful of one-level
   listings, at most once per coalescing window.

If per-directory targeting is ever wanted, the honest trigger is a measurement — N `dir_list` calls
per event becoming visible on a real workspace — not an anticipation of one.

### Q3 — no `decisions.md` entry; two doc edits and one recorded instruction

No product decision changed. W-6 exists as a *Should*; this is it being implemented earlier than
planned. A new D-entry would imply a decision was taken where in fact an existing requirement was
rescheduled.

What should be recorded:

- **The scope move**, v0.3 → v0.1, with the CEO's report as the reason (plan + `scope-mvp.md`).
- **§6's "not yet wired" sentence**, amended, or it becomes a false statement about the code.
- **The CEO's simplicity instruction, once**, as the standing reason the button was declined —
  including that it was declined *on their own stated grounds*, since that is the part a later
  reader will otherwise assume went the other way.

---

## Amendment — the rename, after QA's correction

QA is right that `tree:changed` has a working consumer, and the correction sharpens the scope
point rather than weakening it. Verified: the emit sits directly above
`app.state::<FileIndex>().invalidate()`, and across all seven backend events **`tree:changed` is
the only one with no frontend listener at all.**

**So one consumer is not a half-finished wire — it is the v0.1 design executed correctly.** The
event was connected to the consumer v0.1 needed (quick-open's cache, which could not ship a
knowingly stale index) and deliberately not to the one deferred to v0.3. Both halves were
intentional. That is the opposite of the wire-format bug in kind, not merely in degree.

### Rename: yes. Not to a cause-name

QA's rule holds and I accept it (below). But the obvious application of it is wrong, and the
argument that shows why is one the leader half-made and drew backwards.

`workspace:changed-on-disk` is the natural cause-name — it parallels `document:changed-on-disk`
exactly, same verb phrase, different subject. And the leader observes that such a name makes a
payload *"the natural shape rather than an addition"*. That is true, and it is an argument
**against** the name: the payload is refused above for reasons the rename does not touch — a
coalesced batch spans many directories, so no single path represents it, and the boolean is the
fold `decide` is batch-shaped to produce. Worse, the sibling it parallels *does* carry a payload,
so the matching name makes the absence conspicuous rather than quiet. A name that invites
something we have deliberately refused is a name that will be argued with every six months.

**Name it for the obligation both consumers share.** Look at what they do: `FileIndex::invalidate()`
and, once connected, `Tree.load()`. Both discard a cached listing and re-derive it. The backend's
honest claim is *"the listing you hold may be stale"* — which the backend genuinely **can**
produce, because it is the authority on whether the filesystem moved, even though it cannot re-list
anything itself.

> **`workspace:listing-stale`**

It satisfies QA's rule: the emitter can produce staleness. It names the shared obligation rather
than either consumer's reaction — which is what makes the gap visible, because *"a
`listing-stale` event with one consumer means one listing is not being invalidated"* is a sentence
someone can check, where `tree:changed` with one consumer reads as fine since quick-open's index is
not a tree. And **it does not invite a payload**, because staleness is binary by nature, so Q2's
answer stays settled instead of being reopened by the name.

### QA's rule holds, and the survey is what licenses it

> An event named for an effect its emitter cannot produce will make upstream tests read as
> end-to-end ones.

Accepted. The restraint in offering rather than filing it was right, and the thing that settles it
is not the single rename: **they checked all seven events and found exactly one outlier.** That is
a population check, not an anecdote — the difference between "this name misled us once" and "one of
our seven names is of a kind that misleads, and here is the test for which". Generalising from one
*fix* is thin; generalising from one *exception in a surveyed set* is not, and the survey is the
part to keep.

### One smaller thing, now that there is a real consumer

`let _ = app.emit(…)` discards its result. Leave it, but say why in a comment rather than letting it
sit unexplained beside a consumer that cannot fail: an emit fails essentially only when no webview
exists — during shutdown, or after the window is destroyed — where discarding is exactly right.
Unexplained, it reads as carelessness in the one place the leader already noticed nothing would
notice.

---

## Second amendment — the scope grant cannot be released, and that is the ruling

Both of you converged on QA's watch condition and I had arrived there too, so take that as
settled from three directions. The unwelding question is the one with teeth, and answering it
turned up a shipped defect that decides it.

### The finding first, because the rulings rest on it

**`FsScope` has no removal API.** Its only mutators are `allow_directory`, `allow_file`,
`forbid_directory`, `forbid_file` — all four *push* onto one of two `HashSet<Pattern>`s, and
nothing in the type ever removes from either (`tauri-2.11.5/src/scope/fs.rs`). Both sets are
monotonic for the life of the process. `is_allowed` (fs.rs:419) checks `forbidden_patterns`
first and returns `false` on a match without consulting the allow list at all.

So a grant cannot be revoked, and a forbid cannot be lifted. The only available "release" is a
forbid, which is permanent and beats every later grant.

**This is already a defect in `rescope_workspace`, today, with neither fix applied.**
`forbid_directory(old_root, true)` (commands.rs:116) pushes the patterns `old_root` and
`old_root/**`. I measured those against the scope's own glob options
(`require_literal_separator: true`): `/a/**` matches `/a/img.png` **true** and `/a/b/img.png`
**true**. Therefore:

- Open workspace `/a`, switch to `/c`, switch back to `/a`. Line 119 re-grants `/a` and `/a/**`
  as allowed — and `/a/**` is still forbidden, so **every image in that workspace is
  unreadable for the rest of the process.** Restart is the only repair.
- A loose document open at `/a/b/note.md` when the workspace moves off `/a` loses its images
  permanently, and re-opening the tab cannot repair it: the repair path only calls
  `allow_directory`.

Under D-4 and I-3 — an app expected to stay open for days — revisiting a workspace is ordinary
use, not an edge. I'd tier this at the severity of the two field bugs and note it is *not* caused
by either one; it is the pairing we were about to hold up as the model of balance.

That also corrects my own earlier section above: I wrote that a scope "may be released on the
same condition" as a watch, costing a visible *Should* when released wrongly. Wrong on the
mechanism. Releasing a scope wrongly is not a visible annoyance; it is unrepairable without a
restart.

### Ruling 1 — unweld them, and the reason is now structural rather than a preference

My earlier argument was that the two halves have different *revocation conditions*. The stronger
statement: they have different **revocation possibilities**. `unwatch` is idempotent, restores the
prior state, and may be called wrongly and then corrected. A scope grant has no inverse.

**A reversible operation and an irreversible one cannot share a lifecycle.** Welding them means
every future change to the pair's release path is written by someone who has verified it against
the half that forgives mistakes. Two calls, two names — and the scope one should read, at its call
site, like something that does not come back.

### Ruling 2 — the watch: QA's condition, and no release path

Condition: *is this path outside whatever we already watch.* Accepted as sent; the `None` case
becoming the general case is the right shape.

**Release: none. Do not release watches on tab close or workspace switch.** Grounds:

- `unwatch` is path-keyed and can have more than one logical holder. A release on close
  reintroduces exactly the collision you and I circled twice — and it is reachable: open
  `/a/b/note.md` with no folder, then open `/a` as the workspace, then close the tab.
  `unwatch(/a/b)` is now wrong, and under QA's condition the second watch was never taken, so
  nothing is left covering it.
- The set QA asked me to bound is: **distinct directories of loose documents opened this
  session.** D-15 bounds loose documents to a handful, so this is small by construction — and
  under QA's condition, in-workspace documents never enter it at all.

That is the bound, and it belongs in architecture §6 as a stated bound rather than as an absence.
If it ever stops being small, the fix is a holder count, not an unwatch on close.

### Ruling 3 — the scope: keeps the workspace, which is where the lock question lands

Your original question 1: **yes, the scope half still wants the workspace root.** Not to decide
whether to watch, but because *"is this path already inside the granted root"* is the question
that keeps the stingy half stingy — and it is the only half that must be stingy, since a grant it
makes wrongly is permanent.

So the watch reads the path and the watch set; the scope reads the path and the workspace root.
You guessed the lock would land on the scope half and it does — on that half only. It is
answerable the established way and needs no new care: copy the root out of the guard in one
`let`, as `workspace_open` already does. The invariant is carried by `rescope_workspace`'s
signature taking owned values, not by anyone navigating carefully.

### Ruling 4 — the scope's release path does not exist, so the specification is what has to move

This is the part I am ruling *only* halfway, because the other half is yours.

Plan §5's *"nothing wider"* and architecture §11's *"nothing else is readable by the WebView"*
describe a boundary that **tightens when a tab closes**. Tauri's `FsScope` cannot express that.
The spec is not merely unimplemented; it is unachievable with the mechanism named to implement
it. QA found the gap; the missing counterpart is not an oversight anyone can supply.

Two routes are achievable, and choosing between them is a decision, so it is yours:

1. **Relax the specification** to what the mechanism can hold: *the asset scope is the workspace
   root plus the directories of loose documents opened this session* — monotonic, bounded as in
   ruling 2, exfiltration closed by the CSP as you established. Cheap, honest, and the second
   layer of increment 5's framing goes on carrying the weight.
2. **Stop using the scope as the gate.** Grant a stable root once, and validate each path against
   the live open-document set before it reaches `asset:` — a check over current state, which
   *can* tighten. Correct, and a real increment.

I lean 1 for v0.3 and 2 if the increment-12 audit wants the boundary to actually mean what it
says, because only 2 makes the sentence in §11 true. Either way `forbid_directory` comes out of
`rescope_workspace` — under route 1 because it is not a release, under route 2 because it is not
the gate. Removing it is also the fix for the shipped defect above, and that is independent of
which route you pick, so it need not wait for the decision.

One thing neither route changes: **the grant must stay conditional.** An unconditional grant is
unrepairable by construction, whichever gate we end up behind.

---

## On authoring

I am not taking either fix. The manager's condition is that I review, you author, QA verifies — a
reviewer who writes the code has reviewed their own work, which is the arrangement's whole point.
Both designs above are specific enough to implement from; if either turns out to be wrong in
contact with the code, that is a second review and I would rather do that than pre-empt it.
