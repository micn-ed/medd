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
correct. The conclusion would have been "this mechanism is entirely uncovered", which was false.
Where a test depends on
a timing window, a race, or an environment property, assert that property rather than assuming it —
otherwise a slow machine or a cold start quietly converts a real test into a vacuous one that stays
green, which is worse than a failure because nobody investigates a pass.

---

## A rule about a class is guidance for new members and an audit of existing ones

**Including when the class is instances of this rule.** The smallest recorded case: a document
said *"nothing can emit a Rust-side event"* in one section and *"nothing can emit this specific
event"* in another. The first was pointed out and corrected. The second — same claim, same
document, same author — was left standing, by the person who had just been told about the class,
in the act of fixing the instance.

So the audit half is not optional and is not satisfied by having understood the point. Fixing where
you were pointed is the default behaviour the rule exists to override.

Only the first happens by default. A constraint discovered while designing something new reads as a
constraint *on that new thing* — and the members that already exist, in the same class, are never
revisited.

That is how medd shipped a folder picker that deadlocked the app. The mechanism — *synchronous
commands run on the main thread* — was written up on this project while analysing a command that
did not yet exist. The already-shipped command in the same class was never checked against it.

**The actionable form: a write-up that discovers a constraint should enumerate the existing members
and state whether each complies.** Not *"commands default to blocking"* but *"…and of the seven
that exist, these two call blocking APIs."*

**And: an observation you attribute to your own tooling is still an observation.** The same deadlock
was *seen* — the app hung, it was written off as flaky automation, and the note stayed private. A
plausible local explanation is cheap, usually right, and terminates the search; that is a good prior
doing its job, not carelessness. The failure was that the observation never reached a surface where
someone holding the other half could meet it. Recording it costs one line.

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

## Run a new detector against known-good source, not only known-bad

A detector written for a specific bug will be run against the bug. That proves it can fire; it does
not prove it fires *for the right reason*. Run it against source you know is clean, and check it
stays quiet.

It matters because a false positive here is invisible in the direction you are looking. A
source-scanning check written on this project matched its own explanatory comments — the test
module's prose contained both patterns it was scanning for — and reported an offender against
**already-fixed** code. Run only against the buggy version, it would have named exactly the command
it was written for and looked correct.

**And guard the guard.** If the scan stops finding candidates at all, the check passes by finding
no offenders among none — precisely the failure it exists to prevent. An empty result is not a
clean bill of health, so assert that the scan found something to examine.

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

## Report the claim, not the instrument reading

*"`wc -c` returned 0 bytes"* invites trust. *"`routing.rs` on `main` is a stub"* invites **how do
you know** — and the natural next sentence names the source, where a byte count of zero for a
68-byte file does not survive being written down beside it.

That is the narrow, usable form of *cross-check surprising results*, and it is worth preferring
because it works without anyone deciding a result was surprising enough to warrant a second look.

**This applies to anything you looked at, not only to things that printed a number.** A screenshot
is an instrument. *"Entering the detached state removes the dirty dot"* was read off one — a
reasonable inference from watching one glyph appear and another vanish, and wrong: there had never
been a dirty dot. Checking the source before writing it up is what caught it. Had the inference
shipped as an observation, someone would have hunted a regression that did not exist while the real
gap stayed hidden behind a plausible explanation for the symptom.

**And beware corroboration, which is the strongest signal available and can be supplied by a broken
instrument.** The claim above had two sources: a valid one, checked separately, and an invalid one
that returned zero because the command was wrong. **They agreed — and the agreement is what made it
confident enough to pass on.** They agreed by coincidence: the broken instrument failed in a
direction that happened to match. Two independent sources concurring is exactly what you would tell
someone else to look for, so this failure arrives wearing the signature of good practice.

(The conclusion was right anyway — the module *was* a stub — which is worse rather than better: an
outcome that vindicates a broken method is how the method survives.)

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

**Why this recurs at each new level rather than being learned once:** diagnosing the pattern in
someone else's test is pattern-matching on a *shape*, and shapes transfer between people easily.
Avoiding it in your own strategy requires knowing whether your own check can run — a fact about the
system, not a shape you can recognise — and that transfers not at all. So expect the vacuity rules
to be applied to tests reliably and to strategies unreliably, unless the reachability question
below is carried with them.

Two things to do with it:

- **Reachability before efficacy.** *Before* asking whether a check would have caught the bug, ask
  whether the check can execute against that code path at all. This is the prerequisite, and it was
  missed here for two messages: the question *was* asked, and answered by reasoning — "breaking
  listener L's call would fail L's test" — which was sound given a premise never checked, that a
  test could reach L. What settled it was reading the runtime's source and seeing which five events
  it emits.

  **An unreachable path makes every criterion look satisfied, because nothing can contradict it.**
  That is the vacuity problem arriving from underneath rather than from the side. Cheap to check:
  for each thing a criterion claims to guard, name the mechanism that would run it. If you cannot,
  the criterion is a description of an intention.

- **Ask what a criterion would have caught, against the specific failures that prompted it.** Not
  "is this a good check" but "would this have gone red on the bug I am writing it because of." The
  answer is often no, and it is much easier to see before the criterion has been satisfied once.
- **When a criterion covers only part of the question, split it and name the uncovered part.**
  *Shaping is unit-verified; hook choice is gesture-verified, and one gesture is deferred* is
  honest. A single claim covering both retires the half nobody can check.

## A mock at a boundary makes that boundary unfalsifiable

A mock *defines* the boundary for every test that uses it. Tests on either side can then only
discover disagreement **within** that side — and a contract between two systems can only be tested
by something that reads from one and asserts against the other's expectation, which a mock is
definitionally not.

**The dangerous part is that tests over a mocked boundary amplify the assumption rather than check
it.** When medd's error wire format turned out to be wrong in both halves at once, six green tests
across four suites were not six pieces of evidence — they were six places that had adopted the same
wrong shape. The count read as confidence and was actually exposure. That inverts the usual
heuristic: **more tests over a mocked boundary means it is less likely anyone looks, not more
likely it is right.**

So: pin the contract on the side that can fail, with an assertion built from the *literal* wire
text rather than from the type. An assertion constructed from the same enum cannot see a renaming —
which is what the failure was. And have the other side point at those tests as the contract's owner
rather than restating the shape.

**Ground the assertion in the consumer, not in the type.** A pin that says *"`Tree.svelte`
compares `entry.kind` against `"directory"`"* and asserts that literal string checks the contract.
One built from the enum restates it — which is the error the original bug was made of, one level
up.

**Each pin must fail for its own reason.** Validated by mutation: dropping the rename attribute
from one type kills that type's pin *and nothing else*. A pin that dies alongside twenty others
tells you something broke, not **what** broke, and at a boundary the name of the thing that moved is
most of the value.

**Audit the whole boundary once, not the instance you found.** Seven types cross medd's, and after
the audit all seven are pinned. Four were correct only *by accident of vocabulary* — single-word
fields make camelCase the identity function — and their tests **say so**, so nobody reads a passing
result as evidence of design.

**And state what the pins do not buy.** They fix the boundary's *vocabulary*, not its *semantics*:
they establish that one side emits `relativePath` and the other reads `relativePath`. They would not
catch a path computed against the wrong root, or a hash of the wrong bytes. "The boundary is
tested" is a larger claim than the pins support, and seven green results will imply it unless
something says otherwise.

---

## A mock of a dependency cannot testify about that dependency

`MockRuntime` and `tauri-runtime-wry` are separate implementations of one trait. A test asserting
that `CloseRequested` arrives before the window is destroyed pins **the mock's** ordering; the
ordering the product depends on is the real runtime's. The result would be green, would terminate,
and would say nothing about the code that ships — **while looking exactly like a guard on the
premise the design rests on.**

That was investigated and declined here rather than built. Where a claim is a fact about a
*dependency* rather than about your own code, a mock cannot establish it, and a test that appears
to is worse than no test. The honest alternatives — reading the dependency's source, and re-reading
it after an upgrade, or a real gesture — are both worse than a test, and both beat manufacturing a
green tick.

**The strongest form of the vacuity discipline is declining to build the vacuous thing when you
could.** Every other instance on this project was caught after the fact. This one was a test that
would have passed, terminated, and been reported as coverage — and the person about to build it
stopped, because they recognised the shape in their own work.

---

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

## A clean reproduction is when you are least inclined to keep asking

Filed as a caution rather than a rule, in the terms the person who noticed it asked for — it does
not reduce to a step you can add to a checklist.

QA re-measured the width fix on a different instrument and got 483 against 484, and 1165 against
1166. That degree of agreement is an invitation to stop: the number is confirmed, the mechanism
looks confirmed with it, and there is no visible loose end to pull. It was only on going looking
for something *else* to check that the real gap surfaced — a floor of 1166 cannot bite a screen
that is 1512 wide, so the measurement was right and the story built on it was not established.

**Reproducing a number and establishing that the number explains the symptom are different acts**,
and confidence in the first does not transfer to the second. From the inside they are hard to tell
apart, because a clean reproduction feels like the end of the enquiry rather than the middle of it.
Three rounds of increasingly precise measurement made the leader *more* sure of a causal claim that
none of the three rounds had tested.

The practical residue is small and worth saying anyway: when a reproduction comes back clean, the
next question is not "is the number right" but **"could this number produce what was reported?"** —
and if the answer needs a condition to hold (a display setting, a window state, a document shape),
that condition is an open question with an owner, not an assumption.

## An honest number degrades one generous reading at a time

A carefully honest figure — *three of four links established* rather than *"it's unverified"* — is
worth more than either extreme, and it erodes in a way that is hard to notice: **each individual
over-read is defensible, and the next one starts from where the last landed.**

It happened here. A second, weaker instrument became available on a link that was **already
established**, and that was reported as the fraction moving. Not absurd — just wrong, and wrong in
the direction that makes the number look better. The next reading would have started from the
improved figure.

Two things follow:

- **Report what changed, not what it feels like.** *"A second instrument on an already-covered
  link"* is the fact. *"The fraction moved"* is an inference, and a flattering one.
- **Distance from the artifact, not generosity, is usually the mechanism.** Someone reporting on a
  document they read a summary of will drift from it — not because they are being kind, but because
  they are a step further from the thing. Each reading is defensible and only the file settles it.
  That is an argument for **whoever holds the artifact doing the checking**, rather than for the
  reporter trying harder.

- **The beneficiary of a generous reading is the right person to check it, and the wrong person to
  rely on for checking it.** On this project the correction has twice come from the person being
  credited — and examining *why* it worked makes the point stronger, not weaker. **Neither
  correction came from being alert to flattery. Both came from happening to hold the disconfirming
  fact**: knowing a table already listed that link as covered, knowing a document had forced a
  question because you wrote the sentence.

  So the failure mode is not reluctance, it is **coverage**. A generous reading of your own work is
  precisely the one you are least equipped to notice, because it agrees with what you would hope is
  true — and what saved it twice was an accident of what someone happened to know. That is not a
  control with a gap in it; it is not a control.

  If you wrote the generous reading, catching it is yours. The times it worked out the other way
  are not evidence the other route functions.

---

## Drift that understates coverage is quieter than drift that overstates it

Both directions happen. They are not equally dangerous.

A stale claim that something is **still open** gets corrected the moment someone tries to do it —
the work itself disproves the document. A stale claim that something **cannot be covered** is never
disproved, because *nobody re-checks a claim that something isn't possible.* It sits, and the thing
it discourages stays undone.

That happened here for **five increments**, on the user-facing surface of the one decision where
autosave can destroy someone's work. A mock's comment attributed its own omission to the platform;
everyone downstream read it as a limitation; limitations get accepted where omissions get fixed.

**So audit claims of impossibility more often than claims of incompleteness.** The first kind is
self-correcting and the second is self-sustaining.

**Beware a reproduction that matches the symptom by a different mechanism.** The folder-picker
deadlock hangs. A test calling the same function also hangs — and *not for the same reason*: the
deadlock is blocking the main thread a panel needs, while a mock app has **no run loop for a panel
to appear on at all**. A bounded test would therefore be measuring "no run loop", and would **go
green the day the deadlock returned.**

That is a new face of the vacuity family and the only one so far identified *before* the test was
written. The tell: the reproduction succeeds without the precondition the bug requires. Ask what
the symptom would be *if the bug were fixed* — if the answer is "the same", you are not reproducing
it.

The honest form of such a limit is neither "no instrument reaches it" nor "nobody has found one".
It is: **the call is reachable, the outcome is unassertable, and the scenario cannot be constructed
because its precondition does not exist in a test.** That is a limit on what can be *concluded*,
not on what can be *called* — and it is the version someone deciding how hard to look actually
needs.

**And an impossibility claim in a *verification* document is self-sealing twice over:** it tells
the reader not to try, and it is written by the person whose job is to know. Nobody audits the
auditor's account of what cannot be audited.

So in any document whose purpose is to say what is and isn't established, **every "cannot" carries
what was actually attempted.** That is what lets the next reader tell a tested limit from an
assumed one. Two claims here went unchallenged for five increments and until someone typed one
command respectively — *"there is no real backend here"* and *"structurally cannot"* — and both
were omissions wearing the costume of constraints.

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

## The write-ups are where the checking happens

Four times on this project, someone broke a convention **while actively applying it** — and every
one was caught by the person who broke it. That is not diligence, and treating it as diligence
would miss the mechanism: in all four cases it was caught while **writing about the rule**, not
while using it.

Explaining a rule to someone else puts your own work beside it in a way that applying the rule does
not. You cannot write *"a document that restates a mechanism goes stale invisibly"* without your
eye landing on the mechanism you restated three paragraphs earlier.

So the review write-ups here are **load-bearing rather than overhead.** They look like
documentation of work already done; they are in fact where a large share of the checking actually
happens, and a process that trimmed them as ceremony would lose the catching along with the prose.

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

## Operational documents reference; they do not restate

A plan or a checklist that **restates** a mechanism has to be updated in lockstep with the design
document that owns it — and there is no link back, so nobody knows to. Twice on this project a
ruling changed, the design document was rewritten, and the operational one kept a row describing
the superseded mechanism. What made it invisible both times is that the stale row was still
**true-sounding and on-topic**: it read as current because it was about the right subject.

The fix is structural rather than vigilance. **A row that says "see architecture.md §3 for the
mechanism" cannot go stale, because it holds no mechanism to be stale.** Keep the *decision* and
the *why* in the operational document — those are what an implementer needs to sequence work — and
let the *how* live in one place with a pointer to it.

This is the shared-predicate rule one level up: **an operational document is a caller, and a caller
that restates the answer instead of asking for it drifts.**

**Exactly one document owns implementation status.** Status is the thing that changes daily, so
every document that asserts it is a drift site — and the ones that assert it *in passing*, while
being about something else, are the ones nobody updates. Here that document is
[todo.md](todo.md); the architecture describes the design, and says nothing about whether it is
built yet.

There is a second reason to concentrate it, better than tidiness: **a document that asserts
implementation status makes someone verify implementation status.** Writing one sentence about
shipped behaviour here forced the question of whether it was shipped — which is how 360 lines of
finished work were found to exist only on one machine, unpushed. That check happens because the
document demands it, not because anyone was being observant, so it is worth having a document whose
job is to demand it.

**And the test for which document owns a limitation: does it expire when we finish the work?** A
platform constraint — an event that cannot fire until a bundle declaration exists — belongs in the
architecture, because it is a fact about the platform and will still be true when everything is
built. *"This module is still a stub"* belongs in status, because it is false the moment someone
merges, **while still reading as current**.

### A boundary is stated once and cited everywhere else

The rule above aims at operational documents restating design documents, and it failed inside its
own target. The asset scope is the worked example, and it is worth keeping because **every part of
this entry was already written when it happened.**

One boundary — which directories medd attends to, and for how long — was asserted in **eleven places
across three documents and two source files**: architecture §6, §11 and the §4 IPC table; plan §5
twice, its watcher row, its open-items row, and increment 12's release gate; D-17's own statement of
the bound; and doc comments on `workspace_open` and the watcher module. A decision retired it. The
design document was corrected the same hour. **Nine of the eleven survived that correction**, and
the last was found only by sweeping for the boundary rather than for any sentence.

The count is the point. Nobody would defend eleven copies of a rule, and nobody chose eleven either:
each was written by someone documenting the thing in front of them, in the words that thing
suggested.

**Restatement does not look like duplication when the words differ.** The eleven divided into three
vocabularies sharing no phrase — readability (*"nothing else is readable by the WebView"*, *"nothing
wider"*), watching (*"the parent directory of each open loose document"*, *"drop a loose-file
watch"*), and revocation (*"revokes it from the old one"*, *"grants are never revoked"*). Every
sweep was run in one vocabulary and blind to the other two. And a reader who finds sites from two of
them reads two sources agreeing, not one source twice.

Why the vocabularies existed is worth noticing: the boundary was two constraints — what may be read,
and what is watched — welded together in the code, so each document described whichever half it
cared about. **Unwelding them revealed that the eleven sites did not agree on which constraint they
were stating.** A restatement can drift from its source; these had also drifted from each other.

Neither a person nor `grep` can distinguish a restatement from an independent claim. That is the
mechanism the earlier framing missed: the stale row is invisible not only because it is
true-sounding and on-topic, but because **its vocabulary hides its parentage.**

**The cost is authority, not staleness.** Staleness is the version where one document is wrong and
gets corrected. Authority is the version where eleven sources assert one boundary and an implementer
reads them as independent confirmations of something no longer true. Corroboration is how careful
people check themselves, and restatements corroborate each other. Here two of them nearly reinstated
a defect that had been fixed the same day — one carrying a plan reference, the other an architecture
table row that an earlier review had independently filed as a missing feature. **Two documents
agreeing with each other and both disagreeing with the rule.**

**The worst form is a restatement that has become an instruction.** A stale description waits to be
read; a stale *requirement* recruits someone to act. Three of the eleven had crossed over: the §4
table told an implementer to release a watch the design forbids releasing; the plan's open-items row
filed two now-intended behaviours as defects to be fixed; the release gate asserted a property that
had been deferred to a later increment, so it would go red on conforming behaviour and be "fixed" by
weakening the test. **Ask of any restatement: if this is stale, does someone change the code?** If
so it is not documentation drift, it is a queued regression.

**Position decides whether a stale claim reads as drift or as an omission.** The doc comment on
`workspace_open` said it revoked the old root; the explanation of why it deliberately does not lived
in the helper it calls. Read in the order anyone reads it, you meet a documented intent, find no
implementation, and conclude the implementation was lost. **A correction placed downstream of the
claim it corrects converts a completed fix into an apparent regression.** The explanation belongs
where the wrong conclusion would be drawn, not where it is technically most accurate.

So: **one document owns each boundary, and everywhere else cites it.** The owner is the document
that would have to change if the boundary changed for a reason of its own — for a security boundary,
the architecture section describing the mechanism. Everywhere else holds a pointer and the *why*,
which is what an implementer needs in order to sequence work and is not a mechanism that can go
stale. This extends the rule above past operational documents: **code comments are subject to it
too**, and two of the nine survivors were comments.

**And sweep for the boundary, not for the files you touched and not for the sentence you remember.**
Checking the file you are about to edit tells you about that file. Grepping the phrase you wrote
finds the restatements that share your vocabulary. Neither is the search a negative claim needs —
see *A negative claim is only worth its search*, of which this is the documentation case.

**This entry is deliberately an extension rather than a new one.** A second heading stating the same
rule in fresh words is the exact failure described above, and it would have been the natural way to
add it.
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

- **A long-lived branch carries the *absence* of every fix landed since it forked, and "take
  theirs" reinstates them.** A merge conflict shows you two versions of a hunk. It does not show
  you that one side is simply older — so the side that *adds* the feature, and therefore looks like
  the authoritative one, is also the side missing everything main learned in the meantime.

  **Where it bites is narrower than it sounds, and knowing where is the useful part.** The
  absences only produce a wrong result on lines *both* sides changed — a conflict, where resolving
  it wrongly reverts main's fix. Where only main moved, a three-way merge takes main's side
  cleanly and the absence costs nothing. So **"take theirs" is dangerous in the conflicting hunks
  specifically, in proportion to how long the branch has been open** — not file-wide, and not
  because the branch is hostile, but because a long-open branch has more lines main has since
  moved. `workspace_pick` is exactly that shape: main changed the attribute, the branch holds the
  old one, and both touch the same line.

  **A two-way diff is not a merge, and reading one as though it predicts a merge is its own trap.**
  `git diff main origin/<branch> -- <path>` against this very file reported **552 deletions**,
  which reads as *the merge will delete `conventions.md`*. It will not. A two-way diff says what it
  would take to turn main into the branch; a merge asks what each side did *since the base*. Here
  the base and the branch are byte-identical (23,908) and only main moved (59,164), so the merge
  keeps main's version without a conflict. That near-miss happened while writing this entry, by its
  author, who had used the same command correctly one paragraph earlier for a different question —
  which is the whole reason it is recorded: **the command is right and the inference from it is
  not, and nothing about the output distinguishes the two.**

  `origin/medd-dev` forked before the folder-picker fix. On main, `workspace_pick` is
  `#[tauri::command(async)]` and a `command_shape` guard enforces that shape; on the branch it is
  plain `#[tauri::command]` and still calls `blocking_pick_folder`, and the guard does not exist.
  Resolving that file the branch's way would have reinstated **the deadlock that hung every copy of
  medd on every click of Open Folder**, and deleted the only test that notices — in one clean merge,
  with no conflict marker pointing at either.

  **The two failures are correlated, which is what makes it dangerous.** The guard would catch the
  reverted attribute instantly, so the bad outcome needs both resolved the same way — and they
  are both in the same file, so "take theirs for this file" does exactly that. A guard living
  beside the thing it guards is inside the blast radius of a single resolution decision.

  Resolve by **reconciling hunks, never by taking a side of a file**, and verify the merged result
  against named invariants rather than against the conflict markers:

  ```
  grep -c '^#\[tauri::command' commands.rs          # 9 commands
  grep -c '^#\[tauri::command(async)\]' commands.rs  # 2 async
  grep -c 'mod command_shape' commands.rs           # guard present
  ```

  Anchor those on `^#\[`, and the reason is the sharpest thing in this entry. The check first
  handed over was `grep -c 'command(async)' >= 2`. Measured against main with **every real `(async)`
  attribute reverted**:

  ```
  real ^#[tauri::command(async)] attributes  ->  0      (the fix entirely gone)
  bare grep -c 'command(async)'              ->  4      (prose only)
  the check                                  ->  PASSES
  ```

  **A verification step that returns "clear" in exactly the state it exists to catch.** Its result
  could not contradict the thing it was checking — the same defect as a test that passes when its
  mechanism never runs, arriving in the very message that warned the guard sat inside one
  resolution's blast radius.

  **Why it inflated is the part to remember: the prose doing the inflating was written by the same
  person writing the check.** The guard's explanation of the rule it enforces, and the §2 rule
  behind it, came from the architect's own rulings; so did the grep. Documenting an invariant
  inflates any loose measurement of it, and the two are usually written *in the same sitting*, when
  the wording is freshest in mind and therefore most likely to match a pattern typed from memory.
  Documenter and measurer are the two roles that would otherwise catch each other, and here they
  were one role.

  So: count **structures, not strings** — anchor the pattern to something the prose cannot
  accidentally satisfy. Anchoring is easy; *noticing that you need to* is the hard part, and the
  person best placed to notice is the one least able to, because they wrote the text that hides it.

- **A worktree stops others touching your files; it does not tell anyone what you are holding.**
  This is the gap the worktrees left, and it is the read-side counterpart to them. An unmerged
  branch holds hunks in files that look untouched everywhere else: `main` is clean, `git status` is
  clean, the file opens clean — and the conflict surfaces later, inside someone else's commit.
  Before editing a file another stream might hold, run
  **`git diff main origin/<branch> -- <paths>`**.

  The architect was told *"dev has the width bug, so you won't collide"* and ran the check anyway:
  `origin/medd-dev` had 47 unmerged insertions in the same file, from increment 10's launch routing
  — nothing to do with the width bug. The hunks were ten lines clear so the merge was clean, but
  that was luck, and the check is what established it rather than assumed it.

  **The assurance was not careless; it named the wrong thing.** *"Dev is working on X"* describes
  what someone is **doing**, and the collision surface is what their branch is **holding**. Those
  are different sets, and they diverge with time — a branch keeps holding its files long after the
  work that created them is finished and forgotten. Merge state answers the question; activity
  does not.

  And a teammate's all-clear does not substitute for the command. The architect nearly skipped the
  check *because* the leader had already given one — an assurance from someone who did not run it
  carries no more information than not asking.

  **And checking the file you are about to edit tells you about that file.** That is the narrower
  correction, and it matters because it is the step that feels like it finished the job. The
  architect established that the other branch's hunks in `App.svelte` were ten lines clear of the
  intended edit and concluded the branch would merge — true of `App.svelte`, and silent on
  `commands.rs` and `main.rs`, which were not going to be touched and so had not been looked at.
  The leader's own attempt to land the branch conflicted in exactly those two files.

  So they are two questions and only one of them was asked: **"will my change merge?" is about the
  files you edit; "will their branch merge?" is about every file the branch holds.** The first is
  what the pathspec check answers, and answering it confidently is what makes the second easy to
  stop asking.

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

- **A missing deliverable is invisible to every review that looks at the change.** Increment 10
  merged with `scripts/medd` and `make install-cli` absent. Nothing was wrong with the diff, the
  merge message, or the tests — all of them described what *did* land, accurately. The absence was
  of a file nobody edited, so it appeared in no diff, contradicted no test, and left the merge
  reading as complete. It was found only by checking the increment against its **criteria** rather
  than against its changes.

  This is a different failure from a bad change, and the usual instruments are all shaped for bad
  changes: a diff shows what moved, a test suite covers what exists, a review reads what was
  written. None of them has a place to put *the thing that isn't there*. The one instrument that
  does is a list written **before** the work, because only that list mentions items independently
  of whether anyone produced them.

  So: **when an increment lands, check it against its definition of done, not against its diff.**
  And when the check is "is X present", say so as a criterion rather than assuming X will be
  noticed by its absence — it will not be.

  The companion case is the same shape one level down: a claim can travel without its evidence.
  `verification-status.md` cited a probe result as established while the probe itself sat on an
  unmerged branch. The claim was accurate, current, and exactly as broad as its evidence — and the
  evidence was not in the repository, which reads identically to a verified claim from the outside.
  **Check that what you cite actually landed**, especially when it landed by a different route than
  the thing citing it.

- **An instrument that silently fails to act reports the null result the experiment exists to
  detect.** Investigating the width bug produced three of these in a row, each returning a
  confident, plausible number: a window resize that did not resize, a `2>/dev/null` that hid a
  failing `git show` and left an empty comparison reading as "identical", and a CSS override that
  lost on specificity to Svelte's scoped class — so both arms of an A/B measured the same state and
  returned a tidy, symmetrical, *identical* table. It looked exactly like a clean null result. The
  kind you would report.

  This is the vacuity family one level out. A vacuous **test** passes because its mechanism never
  ran; a failed **instrument** reports "no difference found" because its intervention never
  applied. Both are silent, both look like success, and the instrument version is worse in one
  respect: a test at least sits in a suite where someone may later mutate it, whereas a measurement
  is usually taken once, believed, and acted on.

  The fix is cheap and separate from reading the result: **confirm the intervention took effect
  before trusting the reading.** Read the property back (`getComputedStyle(el).minWidth` must
  actually say `auto`), assert the resize landed, drop the `2>/dev/null`. And prefer an arm that
  *proves* itself — an A/B where both arms report what they applied is self-checking in a way that
  one where both simply report a number is not.

  The tell to watch for: **an experiment whose two arms agree more neatly than the thing being
  measured should allow.** Agreement is the expected shape of an instrument that did nothing, and
  it is also the most reassuring shape a result can take.

- **An absence has a cause, and the cause determines what to do about it.** The entry above is
  about *finding* an absence. This is the next step, and skipping it wastes work: at least four
  causes are **indistinguishable from inside the repository** and want four different responses.

  | what the repo shows | actual cause | correct response |
  |---|---|---|
  | file not present | never written | write it |
  | file not present | written, uncommitted elsewhere | recover and land it |
  | file not present | deliberately deferred | leave it; check the decision still holds |
  | file not present | blocked on a person | do nothing; find out what unblocks them |

  The instance: `verification-status.md` said the CLI shim *"has not been written."* It had been —
  `scripts/medd`, `make install-cli` and `test-shim.sh` were sitting uncommitted in a worktree
  paused by its owner's user. A stranger acting on that wording writes the shim, and the duplicate
  surfaces at merge time if at all. That is not a documentation nicety; it is wasted work with a
  collision at the end of it.

  **The diagnosis worth carrying: the claim was accurate about what was checked and wrong about
  what it implied.** `scripts/` really did contain only `mutants.sh`. *The repository not
  containing something* and *the thing not existing* are different facts, and only the second is
  what a reader acts on. Reporting the first as though it were the second is the same error as
  reporting an instrument's reading as the claim it was taken to support — one level up, about
  provenance rather than measurement.

  Three instances of it landed in one document, all self-caught, in different sections — which is
  what makes it structural rather than careless. So: **when recording that something is missing,
  record how you know and what you checked**, because "absent from the tree" is a fact about the
  tree, and the reader needs a fact about the work.
