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
  *Should*: images stop rendering, visibly.

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

## On authoring

I am not taking either fix. The manager's condition is that I review, you author, QA verifies — a
reviewer who writes the code has reviewed their own work, which is the arrangement's whole point.
Both designs above are specific enough to implement from; if either turns out to be wrong in
contact with the code, that is a second review and I would rather do that than pre-empt it.
