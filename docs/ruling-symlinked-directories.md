# Ruling — is a symlinked directory part of the workspace?

**From:** principal architect
**In response to:** the tree and quick-open disagreeing about symlinked directories after `b7f8bff`
**Status:** measured against the code at `b7f8bff` before ruling.

**Ruled: follow symlinked directories, in both places** — because a symlink the user placed inside
their own root is an assertion of membership, and **medd already honours exactly that for files.**

But the disagreement is not the one reported, and the difference changes both the fix and its
urgency. I measured all four cases first.

---

## 1. What actually happens

| Case | Tree shows | Tree expands | Quick-open |
|---|---|---|---|
| Symlinked dir → **inside** the root | `Directory` | **yes** → its documents | skipped — *but the same documents are found by their real path* |
| Symlinked dir → **outside** the root | `Directory` | **refused** — `OutsideWorkspace` | skipped |
| Symlinked `.md` → inside | `Markdown` | — | **found** |
| Symlinked `.md` → **outside** | `Markdown` | — | **found**, and opens as a loose tab (D-15) |

Three corrections to the framing follow from that.

**The case the product argument is about — `ln -s ~/notes ~/project/docs` — the tree already
refuses.** `Workspace::dir_list` canonicalises and rejects anything not prefixed by the root, and
has since increment 3, with a test asserting it. So for the motivating case the tree and quick-open
*agree*: neither gives you the files. The tree just agrees **badly**, by displaying a directory
that errors when clicked.

**The disagreement that does exist is the harmless one.** For a link pointing *inside* the root,
quick-open still finds those documents under their real path. Nothing is unreachable. Arguably the
walk is right and the tree is wrong here — offering one document twice in Cmd+P under two paths
would be worse than offering it once.

**And the real inconsistency is files versus directories, not tree versus quick-open.** A
symlinked `.md` file pointing outside the root is admitted to the workspace *by both* — it is
listed, it is offered in Cmd+P, and it opens. A symlinked *directory* of the same documents is
refused by both. Same user act, same intention, opposite answers, and nothing decided either. That
is the asymmetry worth fixing, and it is older than quick-open.

---

## 2. The ruling, and the distinction it rests on

**Follow them.** Three reasons, in order of weight:

1. **medd already does it for files.** Case 4 is a symlink pulling an out-of-root document into the
   workspace, today, by design. Refusing the directory form is not a decision anyone made; it is
   `dir_list`'s prefix check catching something it was not aimed at.
2. **The user's act means membership.** D-4 says the workspace is a root folder. Someone who runs
   `ln -s ~/notes ~/project/docs` has stated that those notes are part of this project. That is the
   same assertion as putting the files there, made in the way the filesystem provides for it.
3. **"Don't follow, make the tree agree" trades a visible failure for an invisible one.** It is more
   honest than the current state, and that is its whole appeal — but it silently discards content
   the user explicitly placed in their workspace, with no way for them to tell medd is ignoring it.
   Between *visible but broken* and *silently absent*, neither is good, and a third option exists.

### The distinction that makes it safe

increment 3 scoped `dir_list` because "the tree has no business browsing elsewhere" — and it was
right, about the threat it was aimed at: enumerating your way to `/etc` by constructing paths. A
symlink is not that. So:

> **The guard's job is to stop path *construction* escaping the workspace, not to stop the user's
> own symlinks from being followed.** `..` is construction. A symlink is content.

That gives a rule that is auditable by reading rather than by tracing: **reject any path containing
a `..` component, require the path to be lexically under the root, and then follow whatever links
are actually there.** `Path::components()` makes the `..` scan trivial, and it closes the escape
increment 3's canonicalisation was protecting against — `ws.root().join("../outside.md")` is
refused on the component scan, before any link is resolved.

The surface this widens, stated rather than quietly accepted: a compromised renderer could
enumerate outside the root **only where the user has placed a symlink**. That is far narrower than
arbitrary browsing, it is content the user chose, and `document_read` already accepts any absolute
path by design (D-15) — so this does not open a category that was closed.

### Cycle safety, and what replaces the type-level guarantee

Dev's `DirEntry::file_type()` makes cycles structurally impossible, and that is genuinely the
better shape — I said so and I still think so. Following gives it up, so the replacement has to be
unconditional rather than careful:

**A visited set of canonical directory paths.** Push `canonicalize(dir)` before descending; skip if
already present. That is correct for cycles (a cycle revisits a canonical path), correct for
diamonds (two links to one directory list its documents once, which is also the right answer for
Cmd+P), and costs one `canonicalize` per directory in a walk already doing far more I/O than that.

### One test-design note, because the obvious test is the wrong one

**A cycle test fails by hanging, which is the worst failure mode a suite can have** — it does not
report, it consumes CI, and it looks like an infrastructure problem. Do not write it as the primary
guard.

Test a **diamond** instead: two symlinks in different directories pointing at the same real
directory. It terminates with or without the visited set, and without it the walk lists those
documents twice — so the assertion is a deterministic count, and removing the visited set kills it
fast. Same mechanism, a test that can fail safely. A cycle test is worth having as well, but as the
second one and with the knowledge that it hangs when it fails.

---

## 3. Sequencing: decide now, implement in v0.2 — with one cheap thing now

The ruling should be recorded now; the implementation is a visited set, a guard change and its
tests, and it belongs with v0.2's Finder work where the rest of "what is in the workspace" is
being settled. Symlinked directories are a genuine edge case and the leader is right that this is
not a blocker.

**But one part is near-free and should not wait: stop the tree showing a directory it will refuse
to open.** That is the actual user-visible defect, it predates quick-open, and it is what makes the
current state dishonest rather than merely incomplete. Classifying an out-of-root symlinked
directory as `Other` — visible but inert, exactly as W-8 already treats non-`.md` files — makes
the tree and the walk agree *and* tell the truth, with no new mechanism and no change to the
security posture. It needs the root, so it belongs in `Workspace::dir_list` rather than the free
function.

That is a product-visible change, so it is the leader's to approve rather than mine to land.

---

## 4. Separately: the seventh instance, fixed in this branch

`walk_markdown_files` landed with its own `name_str.to_lowercase().ends_with(".md")` — eleven lines
below `workspace::is_markdown`, in a file that already imports from `workspace`. The predicate was
extracted precisely to stop two places deciding this and drifting.

Behaviour is identical today; the two expressions are character-for-character equivalent. **That is
why it was worth fixing immediately rather than when they differ** — nothing observable changes, so
nothing will prompt anyone to look again.

Fixed in `e0822cf`, with a test that mirrors `the_sweep_and_the_tree_agree_on_what_markdown_is`:
it asserts that quick-open offers *exactly* what the tree classifies as Markdown, rather than that
`SHOUTING.MD` happens to be offered. Making the shared predicate case-sensitive kills it. 96 Rust
tests, clippy, fmt.

Worth noting where the pattern actually recurred: the same commit that *extracted* the shared
ignore predicate across three call sites introduced a fresh literal for a different predicate in
the new code. Sharing one and duplicating another in one change is not carelessness — it is what
happens when the rule is "share predicates" rather than "a predicate has one owner". The second
phrasing is checkable while writing; the first is only checkable while reviewing.

---

# Amendment — ruling on the predicate, and a change of sequencing

**Asked:** rule on the predicate as well as the semantics, and say if the two consumers genuinely
need different questions.

Two answers below, and then a sequencing change, because **QA's red agreement test turns out to be
a forcing function** and taking it seriously produces a better plan than the "defer it all" I gave
above.

## A. No to a shared classifier. Yes to a shared predicate

The two consumers do **not** ask the same question, and it matters:

- `dir_list` produces `TreeEntry { name, path, kind }` where `kind ∈ {Directory, Markdown, Other}`.
  It is answering **"how should this entry be presented in the sidebar?"** — three-way,
  presentational, and it must classify *everything*, including non-Markdown files as `Other`
  (visible but inert, W-8).
- The walk asks two two-way questions: **"do I descend into this?"** and **"is this a document to
  offer?"** It has no use for a third category and should never see one.

So a shared `classify_entry -> EntryKind` would be the wrong extraction. It would hand the walk a
sidebar category — `Other` — that exists for a UI concern the walk knows nothing about, and the
walk would have to ignore it. That is coupling, not sharing.

**But there is a genuinely shared atom, and it is exactly where they disagree:** *does this entry
resolve to a directory, for the purpose of deciding whether medd treats it as a container of
workspace content?* One question, one answer, two callers. That is the same granularity as the two
precedents — `is_markdown` and `is_ignored_name` are both **predicates**, not classifiers — so this
is the same move a third time rather than a bigger one.

**Name it `resolves_to_directory`, in `workspace.rs` beside the other two.** Not `is_directory`:
the one word that carries the ruling is *resolves*, and the failure mode being guarded against is
someone "simplifying" it to `entry.file_type().is_dir()`. A name containing *resolves* makes that
substitution visibly wrong at the call site, which `is_directory` would not.

### And the set now has a name, which is the part worth generalising

`is_ignored_name`, `is_markdown`, `resolves_to_directory` are not three helpers that happen to live
together. Together they **are medd's definition of what a workspace contains.** Every instance of
this bug has been a new consumer re-answering one of them, and a named group is far easier to check
a new consumer against than three scattered functions:

> **The workspace predicates.** Every question about what medd considers part of a workspace is
> answered exactly once, here. A caller that needs one of these answers calls it; a caller that
> computes it again is a bug, even when it computes the same thing.

That last clause is the one that would have caught `e0822cf` at review time, where the duplicated
literal was character-for-character equivalent to the shared function.

## B. Why the same commit shared one predicate and duplicated another

Worth recording, because it is not carelessness. `b7f8bff` extracted `is_ignored_name` across three
call sites *and* introduced a fresh `.md` literal eleven lines below `is_markdown`.

That is what happens when the rule is **"share predicates"** rather than **"a predicate has one
owner."** The first is a thing you notice while reviewing someone else's diff; the second is a thing
you notice while writing your own, because it asks *who owns this answer* before you write the
answer down. Same content, different moment — and only the second fires in time.

## C. Sequencing: QA's red test is a forcing function, and it changes my answer

I sequenced the whole thing to v0.2. That was wrong, and QA's test is what shows it.

**Their agreement test cannot go green at the current semantics** — not by any cheap fix. The
`Other`-classification patch I proposed above fixes case 2 (out-of-root), but case 1 (a link
pointing *inside* the root) still disagrees: the tree follows it, the walk does not. The only ways
to green are to implement a ruling or to adopt the option I rejected. So a deferral means **a
known-red test parked on a side branch for a release**, and known-red tests rot.

But the expensive part of my ruling is not the part agreement needs. Separating them:

- **Cheap, and all that agreement requires:** both sides follow symlinked directories *within* the
  root, and both refuse ones resolving outside it.
- **Expensive, security-adjacent, genuinely v0.2:** relaxing `Workspace::dir_list`'s guard (the
  `..`-scan-plus-lexical-prefix rule in §2) so that out-of-root symlinks work at all.

So there is a clean intermediate I missed:

**Land now.** `resolves_to_directory`; the walk descends when it is true *and* the canonical target
is under the root; a visited set of canonical paths for cycles and diamonds; and `dir_list`
classifying an out-of-root symlinked directory as `Other` rather than `Directory`. All four cases
then agree, QA's test goes green immediately, and the tree stops displaying a directory it will
refuse to open.

**Land in v0.2.** The guard relaxation, which flips both consumers together — and because the
answer lives in one predicate plus one guard by then, it is one edit and the agreement test stays
green across it. That is the payoff of (A): the semantic change becomes a single decision expressed
in one place, exactly as the leader said it would.

The honest description of the intermediate is *"medd follows the symlinks you put inside your
workspace, and does not yet reach outside it"* — which is a sentence we can write in the README,
where *"it shows the directory but cannot open it"* is not.

## D. On the agreement test's character after the fix

The convention is right that the test should become redundant-but-cheap. One refinement: its
character changes rather than its value.

Before the shared predicate it detects **drift** — two mechanisms diverging. After, the only way it
can fail is if someone **stops calling** the predicate. That is still worth catching and it is the
cheapest possible guard, but it means someone will eventually find a test that "can't fail" and
delete it. Its doc comment should say which failure it now guards, so the answer to *"why is this
here?"* is in the file rather than in this document.

**I have not implemented the landable package** — `dir_list`'s classification is product-visible and
I said above it needs the leader's approval rather than mine. Say the word and it is a short branch:
the predicate, the visited set, the two call sites, the diamond test, and QA's agreement test going
green with it.
