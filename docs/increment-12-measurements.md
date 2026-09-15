# Increment 12 — what would count as a pass

**From:** principal architect
**For:** [plan-v0.1.md](plan-v0.1.md) §12
**Status:** written before the measurements are taken.

§12 says *measure this* four times and says what would count as a pass once. A measurement without
a pass criterion is not better than the estimate it replaces — it is **worse**, because the number
looks earned. "We benchmarked it" retires a question that "1 MB is a guess" leaves open.

So: an observable, a procedure, and a threshold for each. One rule applied throughout, because
otherwise this document is just estimates with more ceremony:

> **Every threshold below is derived from an existing constant, or from a cited perception figure,
> or explicitly labelled a judgement.** A threshold invented during the measurement is the
> measurement agreeing with itself.

---

## 1. Large-document thresholds

**The claim.** N-3: *"Keystroke-to-preview latency — imperceptible on typical documents; must not
degrade badly on large ones."* Neither "imperceptible" nor "badly" is falsifiable, so §8's 1 MB /
10 MB bands currently rest on nothing measurable. The bands are the *answer* to N-3; they cannot
also be its criterion.

**The lower band exists because renders queue.** The preview re-renders on a debounce. If one
re-parse takes longer than the debounce interval, a user typing continuously produces renders
faster than they complete and the preview falls behind without bound — which is the actual failure,
not slowness. So the boundary is derivable from a constant we already have rather than chosen:

- **Observable:** wall time for one full pipeline pass — markdown-it parse, highlight, DOMPurify,
  DOM insertion — at a given document size. Measure the whole pass, not the parse: three of those
  four stages are where the cost has historically hidden.
- **Procedure:** a corpus of real Markdown at 100 KB, 250 KB, 500 KB, 1 MB, 2 MB, 5 MB — concatenated
  real documents, not generated filler, because highlight.js cost depends on how much of it is
  fenced code. Ten passes each, report p95. In the real WebView, not the harness: this is engine
  cost and the harness is Blink.
- **Pass:** the lower band sits at the size where **p95 pipeline time reaches 50% of
  `AUTOSAVE_DEBOUNCE_MS`** (500 ms of the current 1000). Half, not all, so a slower machine than the
  measuring one still does not queue.
- **On failure:** if 1 MB is already past that, the constant moves down — the band is the finding,
  not the failure.

**The upper band is about the editor, not the preview**, and needs its own number. Above it the
document opens read-only with no preview, which is a claim about CodeMirror rather than markdown-it:

- **Observable:** keystroke to visible character echo in the source pane, with the preview
  disabled.
- **Pass:** **under 100 ms**, the conventional threshold for an interaction reading as instant
  (Card/Nielsen; cited rather than invented). The upper band sits where echo crosses it.
- **On failure:** the band moves. If echo is still under 100 ms at 10 MB, the upper band is
  unnecessary and should be **removed rather than kept as insurance** — an unused degradation path
  is code that will rot untested.

**One trap worth naming.** Measuring with a file that is mostly one enormous paragraph and
measuring with one that is mostly fenced code give very different answers, and the second is the
realistic one for this product. If the corpus is all prose, the thresholds will be too generous.

---

## 2. Memory soak

§12 already has a criterion — *resident at 8h no more than 10% above resident at 5 minutes* — which
is why this section is about the two things that make it unfalsifiable in practice.

**Measuring medd's process excludes most of the footprint.** WKWebView runs page content in
separate WebKit processes (`com.apple.WebKit.WebContent`, plus networking and GPU helpers), so
`ps` on medd's pid misses the majority of what N-1 is about — and N-1 itself says *"the WKWebView
baseline is 50–80 MB of that"*, so the design already knows the WebView dominates. A green number
from the wrong pid is the most expensive possible outcome here.

- **Procedure:** sum resident memory across medd **and every WebKit helper process it owns**,
  identified by parent pid rather than by name — the names are Apple's and can change. Record the
  process list at the start of the soak and check it is what you expect, rather than trusting this
  document's names.

**10% of what.** A 10% growth criterion passes at any absolute level. N-1 also sets **under 150 MB
with a workspace and a handful of tabs**, and that target is not in §12 at all. Both are needed:
growth catches leaks, the absolute catches a design that was never within budget.

- **Pass, both required:** resident ≤ 150 MB at the 5-minute mark with ten tabs open; resident at
  8h ≤ 110% of the 5-minute figure.

**And an idle soak tests almost nothing.** Ten tabs open and untouched for eight hours exercises no
allocation churn, which is where retention lives. The soak needs the activity the product
actually sees:

- **Procedure:** a scripted loop over the eight hours — switch tabs, type and let autosave fire,
  trigger external changes (a background `git checkout` or a loop touching files), open and close
  documents. I-3 names *"many opened tabs"* as its own concern, so **tab churn is a separate run**:
  open and close 200 documents in sequence and confirm resident returns to within 10% of where it
  started. That is the one that catches a retained `EditorState`, and the ten-tab hold cannot.

---

## 3. Cold start

**The claim.** N-2: *under ~1 second to a usable window.* The threshold exists; **the observable
does not.** "Usable" could mean the window is visible, the frontend has mounted, the tree has
rendered, or a document can be opened — and those are hundreds of milliseconds apart. Increment 1
measured *"cold start to visible window"* when the app did nothing, so comparing today's "usable"
against that baseline compares two different quantities.

- **Observable, two of them, reported separately:**
  - **To visible window** — comparable with the increment-1 baseline, and only meaningful against
    it.
  - **To `frontend_ready()` returning** — the moment medd can accept a routed open, which is what
    increment 10's pending-open buffer waits for. This is the one N-2 should be read as, because it
    is the first moment the app can do what the user launched it to do.
- **Procedure:** launch the built bundle, not `make dev`. Ten launches, report median and worst.
  State whether the run was cold: the OS caches aggressively, so the first launch after a build or
  a reboot is the pessimistic case and every later one flatters. **Report the pessimistic one**, or
  the number describes a machine that has already run medd.
- **Pass:** median to `frontend_ready()` under 1 s, and worst case stated rather than hidden.
- **On failure:** the budget is dominated by WebView startup and bundle parse (§8), so the lever is
  bundle size, which is measurable in the same pass and worth recording alongside.

---

## 4. The quit ceiling

§12 now carries the budget and its worst case. What it lacks is the criterion: *"measure it the way
the large-document thresholds are measured"* inherits an empty one, since those had none either.

- **Observable:** wall time from `app:before-quit` being emitted to `quit_ready` arriving.
- **Procedure:** ten dirty tabs — §8's memory target uses ten, so the numbers stay comparable —
  each with unsaved edits, quit, under concurrent filesystem load (`git checkout` over a large
  repo, or Spotlight reindexing). That load is the worst case the budget already names, so it is
  part of the measurement rather than an adverse condition.
- **Pass:** p95 completion **at or under 1 s**, one third of the 3 s ceiling. That is not a new
  number: 3 s was chosen as *≈3× a pessimistic ten-tab flush*, so measuring ≤ 1 s is checking the
  claim the constant was set from. If the flush takes 2 s, the ceiling was not 3× anything.
- **On failure, two different answers and they are not interchangeable:** if the flush is *slow*,
  the ceiling rises. If it is *variable* — some runs fast, some near the ceiling — the
  progress-reset refinement §12 already records is the fix, because variance means the flat bound
  is bounding total work while the hazard is stall.

---

## 5. The walk on a realistic workspace

Not currently in §12, and it has the cheapest and most durable number of the five.

**Measured on medd's own repository, now:**

```
enumerated with dotfiles hidden only         42,389 files
enumerated with the shared ignore predicate     123 files      (345× fewer)
  of which .md, i.e. what Cmd+P offers           31
walk wall time, warm cache                 0.58s -> 0.25s
```

- **Observable:** files enumerated, `.md` files indexed, and wall time to a complete index.
- **Procedure:** medd's own repository is the realistic workspace — a Rust-plus-Node project is
  exactly D-4's premise, not an adversarial case. Run warm and cold (`sync` then a cold cache, or
  a directory not touched since boot); report both, because a first Cmd+P after launch is cold.
- **Pass:** complete index **within 200 ms warm**, and the enumerated count within a small factor
  of the `.md` count. The 200 ms is a judgement, labelled as one: it is the point at which a user
  who presses Cmd+P and immediately types would see the list settle under their first keystroke.
- **The durable part:** the enumerated-versus-indexed ratio is a regression guard that costs one
  command to re-run. 42,389:31 was the bug; 123:31 is the fix. A future ignore rule that
  accidentally stops applying shows up as a number, not as a slow dialog someone eventually
  mentions.
- **On failure:** if the cold walk is far outside the budget, the answer is progressive results
  rather than a faster walk — the dialog showing what it has while the index completes. That is a
  design change, so it should be a decision rather than an optimisation.

---

## 6. What else is missing from §12

Briefly, because each is small and none needs a section:

- **N-6, "works fully offline", is an *Absolute* with no test.** Run the built app with networking
  off and confirm it functions completely. One run, and it is the only check of a requirement
  marked absolute. Cheap enough that its absence is the only surprising thing about it.
- **N-3 itself is never measured, only its consequences.** §1 above derives the thresholds from
  it; nothing measures keystroke-to-preview on a *typical* document, which is the half of N-3 that
  says "imperceptible". Same 100 ms figure, on a 20 KB document — the size the product is actually
  for.
- **The `.medd-*.tmp` sweep has no measurement, and should keep none.** Its cost is one `read_dir`
  per directory per session, bounded by construction. Recording that it is deliberately unmeasured
  is worth more than a number here.

---

## On the shape of this list

Four of these six were *"measure this"* with no pass criterion, and one was absent. That is the
same failure the project found in `own_write_produces_no_notification`, in the per-listener
criterion, and in §9's *"must not block the dialog opening"* — **a check whose result cannot
contradict anything.** An unfalsifiable requirement and an unfalsifiable measurement fail
identically; the measurement is just more expensive.

The reason it recurs here specifically is worth naming: a measurement *feels* like the rigorous
option, so it does not attract the question a guess attracts. "1 MB is an estimate" invites
challenge. "We measured 1.3 MB" does not — even when nobody wrote down what the measurement had to
show.
