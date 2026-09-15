# medd — working conventions

Practices that outlive any one version. Everything here was paid for: each one exists because
something went wrong that it would have caught, and the incident is named so the convention can be
argued with rather than merely obeyed.

---

## State what a fix must guarantee, before writing it

Before writing a fix, write down the property it must establish — as a statement about the system
that is either true or false, not a description of the change you intend to make.

**Why.** Two sessions independently produced fixes for adjacent bugs in the same code, and the
fixes would have silently cancelled each other. Described as code changes — *"cancel the pending
timer when a tab closes"* and *"ignore the result of a write whose tab moved"* — they read as
complementary. Described as invariants they were obviously in tension, and the tension named its
own resolution: one is about **issuing** a write, the other about **applying** its outcome, and
separating those two made both correct at once.

The same move found a third case nobody had enumerated. It did not come from reading the diff. It
came from asking *what else could owe a write at close time* — which is a question you can only ask
once the property is written down.

**The practice.**

1. Write the property first, as an assertion about the system.
2. Write it beside any invariant the same code already claims. Contradictions surface immediately.
3. Then ask what else could violate the property — not what else resembles the bug.
4. Put the invariant somewhere durable: a test name, or a comment next to the code that maintains
   it. A fix whose invariant is never recorded gets re-broken by the next person.

**The corollary, and it has bitten.** A comment asserting an invariant that nothing enforces is
worse than no comment, because it reads as a guarantee. `tabs.svelte.ts` carried the line *"nothing
here is unsaved in any sense autosave hasn't already made safe, and keeping that true is doc/'s
job"* — a correct statement of intent, which the other module never upheld, sitting directly above
the code that lost data. **If you state an invariant, something must enforce it or test it.**

---

## Assert on values, not existence — and check the assertion matches the name

A test whose assertion is narrower than its name certifies something it never checked, which is
worse than having no test, because it stops anyone looking.

Three instances on this project:

- `own_write_produces_no_notification` asserts per-path classification and never the emitted-event
  decision. It passes while the behaviour it is named for is violated.
- Path-escape tests were written against a temp directory reached through a symlink, so they would
  have passed against a *broken* implementation — failing for a reason unrelated to what they hunt.
- Image tests assert the rewritten `src` — correctly, which is why the rewrite is right — and say
  nothing about the neighbouring `alt`, which was silently empty for five increments.

**The practice.** Where a test's name makes a claim, check the assertion actually establishes that
claim. Then break the code deliberately and confirm the test fails **for the right reason** — not
merely that it fails. Nearly every real defect found here was caught that way.

---

## Measured, not estimated — and validate the instrument

**Why.** Reading mode's column width was specified in `ch`, which is the advance width of the "0"
glyph and not the width of an average character. `70ch` rendered at 91–96 characters. The first
correction was measured rather than estimated — and still landed wrong, because the instrument
averaged in ragged final lines and under-reported at both ends. Three separate measurements were
needed to settle one number.

**The practice.** Measure. Then calibrate the instrument against a value the project already trusts
before believing what it says about a new one — that is what made the third measurement actionable
rather than merely another opinion. And record the calibration, not the conclusion: a number
without its method gets "corrected" back by the next reader who thinks it looks wrong.

---

## Authorship and acceptance are separate

Whoever writes an increment writes its tests too — that is how correct code gets written, not a
verification step, and in practice it is where almost every real defect here has been caught.

Acceptance is somebody else's: an independent pass against the increment's definition of done,
adversarial where it can be, looking for what the author and the reviewer both missed. Neither
substitutes for the other, and collapsing them loses the half that finds things.

**Report what held up, not only what failed.** An acceptance pass that lists only findings is
half a result — knowing which orderings were traced and survived is what tells the next person
where *not* to spend a week. Revising your own finding downward against its own evidence is worth
more than the finding was.

---

## Sharing a checkout

- **The working tree belongs to whoever is mid-increment.** Reviews, documentation edits and
  exploratory work wait for the gap between increments, or happen in a copy outside the repo.
- **Test commits, not working trees, and say which revision a result refers to.** The rule above
  protects *writes*; this is its read-side counterpart, and it was learned the expensive way. A
  verification run against a tree someone is actively editing is a snapshot of a revision that may
  never have been committed — two runs can disagree and both be correct against superseded states.
  It very nearly produced a confident report of a defect that had already been removed while the
  test was running.
- **Commit explicit paths, never `git add -A`.** A blanket add sweeps up someone else's
  uncommitted work in progress — which at best produces a commit whose message does not describe
  its contents, and at worst loses that work to a later reset. Check `git status` before
  committing and stage only what you changed.
