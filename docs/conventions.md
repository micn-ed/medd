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

**The question that finds the rest: would this test still pass if the mechanism simply didn't run?**
It has one tell in common across every instance found here — the test passes in a world where the
thing under test never happens. And it is answerable by *mutation* rather than opinion: disable the
mechanism a suite is named for, and every test that stays green was never testing it.

Run against this project's own verification suites, that found four vacuous assertions in one pass:
a pair named for cursor preservation across an external reload, which passed with the reload
disabled because a document nothing changed keeps its cursor; one asserting no carriage return
reaches the buffer, equally true of a buffer nothing touched; and one comparing the tab state
produced by the two routes into a conflict, which with conflict-raising disabled compared two
identical *absences* and found them equal.

That last one carries the sharpest lesson, because **the thing that made it feel rigorous is what
made it vacuous**: it compared whole state snapshots rather than spot-checking fields. Comparing
everything sounds stronger than comparing something, and here it meant comparing two empty things
and finding them equal.

Two corollaries worth keeping:

- **Precision about method is not the same as the method being sound**, and the difference is not
  visible by introspection. The tests most likely to be vacuous are the ones their author would
  defend hardest, because confidence and thoroughness feel alike from the inside.
- **Distinguish a wrong claim from weak evidence.** When these four were fixed, every conclusion
  they had been cited for still held. What was wrong was the strength of the evidence, not the
  findings — and saying so precisely is what stops "four of my tests were vacuous" from implying
  something false in the other direction.

When fixing a vacuous test, leave a comment naming the mutant its new assertion kills. Otherwise it
reads as redundant and gets optimised away by the next person.

**A mutation harness must fail loudly when a mutation does not apply.** A mutant that failed to
apply is **indistinguishable from one that survived** — and it fails in the more alarming
direction, sending someone to write a test for a property that is already covered. It happened
here: a script asserted the pattern it was replacing existed, then reported its result anyway, so
when `cargo fmt` reflowed the target across five lines the substitution silently did nothing and
the table printed `*** SURVIVED ***`.

Same shape as *announce the act, not the outcome*: the script **had** the information and the
report discarded it. Non-application is a hard stop, never a row in the results. And note how it
was caught — the survivor was surprising enough to re-check, which is exactly the wrong reason to
catch something.

**Run a mutant against the whole suite, not against the test you expect to fail.** Running it
against that one test tells you *that test passes*. Running it against everything tells you **which
test is carrying the property** — and those differ more often than is comfortable. On this project
a symlink-following mutant survived the test written for it (the document was still reachable by
its real path, so the assertion held) and was killed by a test on the other side of the system
entirely. The property was covered; the map of what covered it was wrong. A mutant run narrowly
would have reported a survivor and sent someone to write a test that already existed.

**Redundant is not vacuous, and only one of them is a reason to delete.** A vacuous test passes in
a world where the mechanism never runs. A redundant test fails correctly — it just fails alongside
others that catch the same mutant. Conflating them argues away useful tests using an argument built
for useless ones.

The operational test: **a vacuous test passes when the mechanism doesn't run; a redundant test
fails when the mechanism breaks, but so do others.** The first is a defect in the test. The second
is a fact about the *suite*, and only a reason to delete if the duplication also carries no other
value.

The distinction that decides it is *form*. A case assertion says the output is a specific expected
value; a **differential** assertion says the output is unchanged by some addition. The second does
not need to know the right answer, only that something shouldn't move it — so it survives changes
to the expected value, and adding the next variation to it is a small edit rather than a new
expected-output test. That is worth keeping even when today it kills exactly the mutants the case
tests kill.

**But be honest about where the generality lives.** A differential test named for a *class* of
routes, whose body exercises one route, is general in its **form and its name**, not in its
mechanism — it will catch the next route when someone adds that route to it, and not before. A
comment claiming otherwise is the same failure as a test name promising more than its assertion
establishes, one level up. The honest version is still a good reason to keep it: *this is the named
home for the class, and it makes the next instance cheap to cover.*

**Ask it of specifications too, not only of code.** A test described in a plan can be vacuous
before anyone writes it. This project's named deliverable for its one unverified path — launch,
type a character, quit, check the file — passed against a quit flush that did nothing at all,
because the ordinary autosave writes the same character a second later and the spec never said the
quit had to happen first. The spec was reviewed by three people and the omission was a *timing
constraint*, which is the kind of detail that reads as implementation noise right up until it is
the entire test.

**And a test must be able to detect that it has stopped testing anything.**

**Measure mutation kills by the *set of failing test names*, never by the count.** Counting is
only valid against a fully green baseline, and a project with legitimately-red tests — a queued
finding, a known defect not yet fixed — silently breaks the arithmetic. It happened here: under
one mutant the failure count stayed at three and read as a clean survivor, while underneath, two
baseline-red tests had flipped *green* (they assert a state clears, which disabling the mechanism
also achieves) and two others had flipped red. Two real kills, perfectly masked, and the sum looked
correct. The conclusion would have been "this mechanism is entirely uncovered", which was false. Where a test depends on
a timing window, a race, or an environment property, assert that property rather than assuming it —
otherwise a slow machine or a cold start quietly converts a real test into a vacuous one that stays
green, which is worse than a failure because nobody investigates a pass.

---

## "I couldn't test this" is usually a finding about the code, not a limitation of the tester

When something resists testing, the first question is not how to reach it but **why it is out of
reach**. Twice on this project the answer was the same structural defect, and both times it was
first reported as a coverage gap:

- `watcher.rs`'s event loop was never executed by any test while holding every observable decision
  — ignore-filtering, the tracked/untracked branch, the `tree_changed` fold.
- The repeat-press behaviour lived in a branch inside the runtime-event handler, so nothing could
  pin it.

Both have the same tell: **the function holding the framework handle was the one with no test.**
That is not a coincidence, it is the rule in `architecture.md` §2 being violated — *a shell
contains no decisions; if a framework-aware function has a branch in it, it is in the wrong place.*
Untestability was the symptom, not the problem.

So the reportable statement is rarely "I can't reach this". It is "there is a decision in a place
that cannot be reached, and here is why that placement is wrong."

These are not the same observation phrased two ways — **they are different instructions to whoever
reads them next.** The first sends someone to build a cleverer harness: an afternoon spent
constructing the scaffolding that exists only because the code is shaped wrong, after which the
defect is still there and now has a test propping it up. The second gets the code fixed and the
harness never needs to exist. The weaker phrasing isn't merely less useful; it is actively
expensive.

---

## Point a detector at the thing, not at a proxy for the thing

A detector keyed to *how* something is currently done goes **quiet, not loud**, when the how
changes. The failure is silent by construction — there is no red test to notice, because the
detector's whole job is to stay quiet until it isn't.

Three near-misses here, all on the same verification watch: it matched a runtime event name, and
the implementation routed around that event entirely; it matched a docs commit that wasn't the fix;
it matched a clean-tree condition that an untracked file could hold false indefinitely. The working
version watches the blob hashes of the two files whose behaviour is under test — because *those
files changed* is the thing actually being waited for.

**Re-ask whenever the implementation approach changes**, which is precisely when nobody thinks to:
the change that makes a detector blind is the same change that has everyone's attention elsewhere.

**And when a detector produces a false positive, fix the verification, not the trigger.** This is a
*different* failure from the one above and a worse one, and it was arrived at while correcting for
the first — so it is recorded separately rather than folded in.

After a watch fired on an unrelated commit, the instinct was to narrow it: key on identifier names
guessed from a design discussion rather than on the file changing at all. **Narrowing feels like
precision and buys silence.** If the implementation had chosen different names, that watch would
never have fired, and the wait would have looked exactly like "not landed yet."

The trade is not symmetric:

> **A false fire costs one verification. A missed fire costs the whole task.**

So prefer the trigger that is guaranteed to fire and cannot be clever — a file changing — and put
the intelligence in the verification step, which is where correctness actually lives and which
happens either way. A cheap trigger plus mandatory verification beats a clever trigger, every time.

Best of all: make the trigger carry its own instruction not to be trusted. A watch whose output
line reads *"verify by running the test — do not infer"* cannot be mistaken for a result by whoever
reads it next, including its author a week later.

---

## A tool whose failure mode is silence must fail loudly when it does nothing

The mutation harness in `scripts/mutants.sh` took five bug fixes to work, and **four of them failed
toward a clean pass.** Under `set -euo pipefail`, a green baseline makes `grep` exit 1; a working
mutant makes the test runner exit 1; `[ -z ] && continue` returns 1 for every real row. Each ended
the script mid-loop, after which it printed nothing and exited 0 — and **for a mutation harness,
"no output, no survivors" is exactly what success looks like.** The tool built to hunt silent
failures spent its first hour being one.

So: any tool whose *success* is reported by an absence needs a guard that fails loudly when it did
no work. The harness now refuses to exit 0 if zero mutants ran.

The fifth bug is the sharper one. The field separator was `|`, which also occurs inside the code
being mutated (`tab.conflict || tab.detached`), so one mutation was split mid-expression and
applied malformed. **A malformed mutation injects a syntax error, fails every test at once, and
reports a huge kill — a false result that looks better than a real one.** It was caught by a
compile check added on general principle an hour earlier, which is the most direct argument for
that check anyone could ask for: verify the mutant is valid code before believing what its failures
mean.

---

## `test.fails()` is the wrong tool for a known-broken behaviour

It passes when the body throws **for any reason at all** — a genuine assertion failure, a call to
an undefined function, and a `throw` in setup are all reported identically as "expected fail". So
it certifies *something went wrong in here*, not *this behaviour is broken as described*. A rename
rots the test into failing for an unrelated reason and it keeps reporting green, which is what you
want to see, so nobody looks.

Its apparent advantage — going red when the fix lands, announcing itself — evaporates too: a test
already failing for the wrong reason keeps failing after the fix, and announces nothing.

**Use a characterisation test instead:** a plain assertion on the current, wrong value, green now
and red the moment the defect is fixed, failing *specifically* on a value mismatch. Say in a header
that it asserts wrong behaviour deliberately, and mark each assertion with what it should become.
The fix is then a mechanical diff.

---

## Put the honest account in the artifact, not in the transmission

A report can be accurate while the thing it reports on overclaims, because they are **different
objects and only one of them ships.** That happened here: a test's mutation result was described
honestly in a message — *"its value is entirely in the route that doesn't exist yet"* — while the
comment inside the file said it already covered those routes. The transmission was right and the
artifact was wrong, which is the wrong way round. Nobody reads the message in six months.

Two consequences.

**Review the artifact, not the description of it.** This is a stronger argument than *trust but
verify*, because the summary can be honest and the thing still be wrong. Verifying the summary
against the reporter's intent catches dishonesty; only reading the code catches this.

**When you write an honest caveat in a message, check it also exists where the work lives.** The
caveat is usually written at the moment of greatest clarity about the work's limits, and that is
exactly the moment it is easiest to spend on the transmission and forget the file.

---

## A verification criterion that seems to guard the failure retires the question

The vacuity rules above are about individual tests. The same failure happens one level up, to a
*strategy* — and there it is worse, because a strategy is written precisely to close a question and
nobody re-opens a closed one.

It happened here. After two bugs in one subsystem, a per-listener criterion was written to catch
the next. Checking whether it was achievable found that **it could not catch either bug it was
written for**: both were about *which platform hook was chosen*, which lives in registration code no
test can reach, while the criterion covers route *shaping*. And for two of three listeners it could
only ever report "untested", because the mock runtime cannot emit their events at all. **A
criterion whose only possible answer is "untested" is not a criterion.**

Two things to do with it:

- **Ask what a criterion would have caught, against the specific failures that prompted it.** Not
  "is this a good check" but "would this have gone red on the bug I am writing it because of." The
  answer is often no, and it is much easier to see before the criterion has been satisfied once.
- **When a criterion covers only part of the question, split it and name the uncovered part.**
  *Shaping is unit-verified; hook choice is gesture-verified, and one gesture is deferred* is
  honest. A single claim covering both retires the half nobody can check.

An end-to-end test exercises the real thing, which makes it feel like the strongest evidence
available. Often it is the *weakest*, because the real thing is nondeterministic and the test can
only fail when the environment happens to cooperate.

Two instances here, arriving from opposite directions:

- A **cycle** test can only fail by *hanging* — the worst failure mode a suite has. The diamond
  test fails as a deterministic count instead.
- A **real-watcher** test could not kill the redundant-ancestor mutant, because whether the OS
  reports a containing directory varies between runs. The pure unit tests on the comparison killed
  it every time.

In both cases the property is real and **the reliable guard is the pure one**. Keep the end-to-end
test — it confirms the pieces are wired together, which nothing else does — but do not count it as
the guard for a property a deterministic test can hold.

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

## Store the inputs, derive the fact

When two or more inputs determine a presentational fact, **store the inputs and derive the fact.**
Storing the fact requires every input's handler to remember to update it, and one of them always
forgets.

Three instances, and the third is what made it a rule:

- **Dirty state** was derived from the start — `currentText !== lastSyncedText`. A derived value
  cannot fall out of sync with reality; a flag can.
- **The line-ending convention** was stored, then deleted in favour of detecting it from bytes just
  proven to be on disk. The stored version created a three-way hazard between `document_close`, the
  close-flush and a write, in which a CRLF file would have been silently rewritten to LF. The
  derived version cannot, because there is no stored value to go stale.
- **Sidebar visibility** was stored as `sidebarCollapsed` and written by *two* concerns — the
  user's toggle, and reading mode. Reading mode remembered to set it; nothing remembered to unset
  it. Once view mode became per-tab there was no single place that *could* have, so one tab's
  reading mode collapsed the sidebar for every other tab.

**The name is part of the rule.** `sidebarCollapsed` reads like the thing on screen, which is
precisely the invitation that produced the bug: someone who wants the sidebar hidden assigns to the
variable that appears to mean "the sidebar is hidden". Name a stored input for what it records —
`sidebarHiddenByUser` — so the next person has to notice it is an answer to a question rather than
the state of the world.

**And check what the change does to the controls.** Deriving a fact can leave a control that writes
one of its inputs looking broken — a button that is present, enabled, and visibly does nothing is a
worse defect than the staleness being fixed, and it will be reported as broken because it is. The
fix is usually to remove the control in the states where it cannot act, not to disable it: a
disabled control needs a visible reason, and a tooltip is not one.

---

## Ask what the symptom will look like, and what people will blame

Bundling related work into one change is normally right. **It stops being right when one item
pre-loads a misdiagnosis of the other.**

Two fixes were queued together here: a latent lock bug, and the new async command that would make
it reachable. Landing them as one change would have been tidy — and the resulting symptom,
intermittent UI stalls, would have looked exactly like the new feature being slow, which is
precisely what everyone was already watching for. The wrong explanation would have been sitting
there, plausible and ready. So the latent fix landed first, on its own, and any stall seen
afterwards is genuinely attributable to the feature.

The question to ask whenever a known-latent fix and a suspicious new feature are queued together:
*what will the symptom look like, and what will people blame?* The fix is cheap now and expensive
once it is competing with a convincing wrong answer.

---

## When two places must agree, test the agreement — not the answer

Where the same question is answered in two places and the *right* answer hasn't been decided yet,
write a test asserting only that **the two agree**, with no opinion about which way. It goes green
whenever both sides say the same thing, so it never needs inverting when the decision lands. That
makes it the right instrument in the case where the detector is ready before the ruling — which is
common, because noticing a disagreement is easy and resolving it is a product call.

Concretely here: the file tree follows directory symlinks and the quick-open walk does not, so a
symlinked directory of notes is visible in the sidebar and absent from search. The test asserts the
two enumerations agree; it deliberately does not encode whether a symlinked directory belongs to
the workspace.

**Write the detector by route, not by case.** *"Walking a root with `node_modules/docs/hidden.md`
must return the same thing whether or not an alias to that directory exists"* is a statement about
a **route into an ignored location**. *"A symlink named `aliased` pointing at `node_modules` is
excluded"* is a statement about one case. The first catches hardlinked directories, a future
include-file, a bind mount — whatever reaches the excluded place sideways next. The second catches
the instance you already found.

This is the agreement test one level out: instead of asserting two consumers agree, assert that
**adding a new path to the same content changes nothing**. Both are decision-independent, and both
survive the fix that prompted them.

**Be explicit about what a green agreement test does not evidence.** It establishes that two sides
answer alike — not *how*, and not that either mechanism works. When symlink-following turned out to
be currently inert here, the agreement test passed either way, which is correct: it asserts the
match, never the mechanism. Saying so prevents the green being cited later as evidence for
something it never claimed.

**A choice is one line over the shared answer; a duplicate is a second answer.** That is the test
for when local logic beside a shared predicate is legitimate. The watcher needs the ignore question
asked of *ancestors only*, while the walk needs the leaf included — so the watcher composes one line
on top of the shared form, named for its reason. It is a decision about *watching* (something
changed inside a directory medd cares about, and *what* changed is not what decides that), not about
workspace membership. If the local part grows past composing over the shared answer into
re-deriving it, it has stopped being a choice.

**And there is a positive signal for successful sharing, not only a negative one for duplication.**
Break the shared predicate and see how many callers fail. When excluding the leaf from
`is_within_ignored` failed tests in *three* modules at once, that was the shape of a genuinely
shared question: **breaking it breaks every caller, rather than breaking one copy while the others
quietly keep agreeing.** The absence of that spread is what duplication looks like from the outside
— and it only shows up if the mutant is run against the whole suite.

**Not everything that looks shared should be shared.** A question two callers both *could* ask is
only a shared predicate if they are asking the same question for the same reason. `dir_list` needs
to know whether an entry resolves to anything at all, because it has a category for the answer
(W-8's visible-but-inert); no other consumer has that category, so that check stays local and is
deliberately not one of the workspace predicates. Sharing it would hand every other caller a
distinction it must then ignore, which is coupling wearing the costume of reuse.

**Where it lives:** with the consumer that was added *later* — that's the side that can drift from
an already-established answer.

**A parked red test is a forcing function, and that is a property of the mechanism rather than of
any one finding.** When this project's symlink disagreement was first ruled, it was deferred to the
next release. What reversed that was not the test's content but the **parking decision**: a
deferral would have meant a known-red test sitting on a side branch for a release, and known-red
tests rot. A red test on a branch is *a claim with a cost attached*, and costs get sequenced in a
way that observations in a document do not.

It cuts both ways, and the caveat is the reason it works: it is leverage **because** parked red
tests are genuinely expensive, which is also the reason not to park many of them. A symbolic cost
would not have moved the schedule.

**Say in the test's own comment which failure it now guards.** An agreement test's *character*
changes once the shared predicate exists — before, it catches two mechanisms drifting apart; after,
the only way it can fail is if someone stops *calling* the predicate. That is narrower and rarer,
which is exactly the shape that eventually reads as a test that cannot fail and invites deletion.
The comment should say what it guards and what deleting it costs — nothing today, and the next
instance later. In the file, where *"why is this here?"* actually gets asked.

**And the test is the detector, not the fix.** A disagreement between two places holding the same
knowledge is structural, and this project's own answer to it is a shared predicate — `is_markdown`
and `is_ignored_name` both exist so their question cannot be answered twice. An agreement test
holds the line until the predicate exists, and should become redundant-but-cheap afterwards rather
than remaining load-bearing. If it stays load-bearing, the underlying duplication was never fixed.

---

## A negative claim is only worth its search

"No reversed lock order found", "no other instance of this bug", "nothing else depends on that" —
these are only worth saying if the search was exhaustive, and worth **more** when you say which it
was. Reporting a negative from the sites you happened to be looking at reads identically to
reporting one from every site there is.

So: state the scope of the search alongside the result. *"Only these two functions take both locks,
and both take them in the same order"* is a different claim from *"the two I looked at agreed"*,
and only the first licenses anyone to stop worrying.

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
  its contents, and at worst loses that work to a later reset.

- **`git add <paths>` is not enough; use `git commit -- <paths>`.** This is the sharper version of
  the rule above, and it was learned by breaking it *after* writing it. `git commit` commits the
  **whole index**, not the paths you just added — so if another session has already staged its
  work, your commit takes that too, however carefully you named your own files. It happened here: a
  commit whose message described a 24-line documentation correction contained 1,174 lines of
  someone else's finished feature work. Nothing was lost, and the history was only honest again
  after being split apart by hand. Naming paths on the `commit` itself bypasses the shared index
  entirely.

  Mind the argument order: **`git commit -m "…" -- <paths>`**, with the message *before* the
  pathspec. Everything after `--` is treated as a path, so putting `-m` after it silently consumes
  the message and the flag as filenames. Found on the first real use of the rule by someone other
  than its author, which is the usual way an under-specified instruction gets found.

- **Work in a `git worktree`, not a shared checkout.** `git worktree add ../medd-<task>` gives a
  separate working directory on its own branch, sharing the same object store — so nothing is
  duplicated but the checkout, and **another session physically cannot stash, reset or check out
  your files.** This is the structural version of every rule below it, and it exists because those
  rules were not enough: the convention against mutating a shared tree was violated twice, by two
  different people who had both read it, within hours of being written. A rule that depends on
  attention fails exactly when attention is elsewhere, which on a shared tree is most of the time.
  Both incidents were recovered only because someone checked the reflog before doing anything else.

  The rules below still apply — a worktree removes the class of accident, not the need for care.

  **Why the prohibition wasn't enough is worth stating, because it generalises.** A rule phrased as
  *don't do X* invites arguing about scope, and a rule phrased as *the purpose is Y* invites
  deciding your case doesn't serve Y. Neither violation here came from looking for a loophole —
  both came from reading for the rule's purpose, concluding in good faith that this case didn't
  engage it, and acting. A structure that removes the judgement needs neither reading.

- **Announce the act, not the outcome.** *"The tree is back how you expect"* and *"I ran
  `git checkout --` on a file in your working directory"* are different sentences, and only one is
  a warning. Stating an effect reads as tidying; it lets the writer feel they disclosed something
  while leaving the reader no way to connect a later surprise to its cause. That happened here —
  a destructive operation was described by its result, in a status paragraph, and the person whose
  file vanished spent time investigating a mystery that had already been "announced".

  The general form: **a disclosure that does not name the action is not a disclosure.** If someone
  would have to infer what you did, you did not say it.

- **If someone else's work disappears, pin it before anything else.** `git tag wip/<what>
  <sha-from-reflog>` makes an orphaned commit permanently reachable and takes nothing from anyone.
  Then tell whoever owns it and let *them* restore it: they know what state they left it in, and
  reaching into someone's half-finished work to be helpful is the same class of mistake that lost
  it.

- **Never `stash`, `reset`, or `checkout` the shared tree to get a clean state — copy it.** Also
  learned by doing. Running `git stash` to get an uncontaminated test read removed a colleague's
  in-flight work from under them mid-edit; it was restored within a minute, and only because it was
  noticed immediately. Every one of those commands is a write to a tree that belongs to whoever is
  mid-increment. If you need a pristine checkout, copy the repository somewhere else and work
  there — which costs seconds and cannot take anything from anyone.
