# Ruling — reading mode keeps the sidebar

**From:** principal architect
**Decision:** the CEO's, not up for debate. This is the design consequence and the record question.
**Checked against:** the code at `60a42e9`.

Three answers and one finding that changes what the decision record should say.

---

## The derivation collapse is right, with two consequences worth taking

Confirmed: the `viewMode` clause drops out of both, and the inert-control problem disappears by
removing the state that caused it.

```ts
showToggle = workspaceRoot !== null
visible    = showToggle && !hiddenByUser
```

**Remove the `viewMode` parameter rather than leaving it unused.** An unused parameter is a place
someone reintroduces a condition without reading the file, which is precisely how the original bug
arrived.

**And keep `sidebarLayout` as a module, even though it is now nearly trivial.** Someone will
propose inlining it — it is four lines and two cases. That would put two expressions back in
`App.svelte`, which is the exact shape that produced the bug: `visible` is defined *in terms of*
`showToggle`, so the coupling is structural rather than remembered. **The function becoming trivial
is not an argument for deleting it; it is what success looks like.** The thing being prevented
costs nothing to prevent.

One test needs re-aiming rather than deleting. I insisted that `viewMode === undefined` be pinned
explicitly — the moment right after *Open Folder…*, before any tab is clicked. That scenario still
matters and is still the first thing a user sees, but it is no longer about `viewMode` at all.
Keep the case, drop the `viewMode` framing, and say in the test why it changed — otherwise it reads
as a test guarding a condition that no longer exists.

---

## 1. Does reading mode still mean anything? Yes — and more robustly than the question assumes

The question was put as *"it still hides the editor and applies the reading measure, so I think
yes."* Reading mode actually does **four** things, and three of them are typographic:

| what | value | shared with split view? |
|---|---|---|
| hides the editor pane | — | no |
| **serif face** | `ui-serif, Georgia, 'Times New Roman', serif` | **no** — split uses the default |
| **larger size** | `17px`, "meant for sustained reading" | **no** |
| **measure cap, centred** | `50ch` with generous padding | **no** — split's preview is uncapped, deliberately |

So reading mode was never primarily *"remove everything else"*. It is a **typographic mode**: the
document set as a page rather than as a pane. Hiding the chrome was one of four distinctions, and
the only one that took something away from the user rather than giving them something.

That does not merely accommodate the decision — **it makes the decision the more coherent of the
two designs.** D-3's rationale is that these documents are read far more often than written, and
all four properties serve that. The auto-collapse was the odd one out.

---

## 2. The D-4 contradiction is narrower than it looks, and was already void

D-4's rationale says:

> Collapsing the tree is what makes reading mode genuinely full-width.

**Reading mode is not full-width.** It is a 522px centred column (50ch, measured). Computed across
every realistic window size:

```
14-inch logical (1470)   sidebar hidden  content 1470px   column 522px   unchanged
                         sidebar shown   content 1230px   column 522px   unchanged
16-inch logical (1728)   sidebar shown   content 1488px   column 522px   unchanged
narrow window    (900)   sidebar shown   content  660px   column 522px   unchanged

the sidebar starts costing the column below a 762px window; medd's default is 1000px
```

**The sidebar costs reading mode's text nothing at any window size medd ships in.** It only shifts
where the centred column sits.

So D-4's stated mechanism stopped being true in **increment 6**, when the measure cap landed — not
today. It was plausible when written, because reading mode was conceived as full-width and became a
capped column later, and nobody revisited the sentence. This is the same drift class the project
keeps finding, in a decision's *rationale* rather than in a design document.

That changes what the record should say. It is **not** "the CEO overruled D-4's reasoning". It is:

- D-4's *full-width mechanism* was superseded by the measure cap and has been wrong for six
  increments;
- D-4's *visual quiet* argument survives on its own terms — a file tree in peripheral vision is a
  real distraction even when it costs the column nothing — and that is the half the CEO has decided
  against;
- so the amendment overrides a live preference and corrects a dead mechanism, and those are two
  different edits.

Recording it as one thing would bury a six-increment-old factual error inside a product change,
which is how a decision doc stops being a record.

---

## 3. Does `sidebarHiddenByUser` persisting across modes surprise? One real cost, and one tempting fix to refuse

**The cost.** The workflow the auto-collapse served — read quietly, then return to editing with the
tree — now costs two toggles where it cost zero. Hide the sidebar for reading; show it again for
split. That is the direct consequence of the decision and I am not relitigating it, but it should
be stated where the decision is recorded, because it is the thing the old behaviour was *for*.

**Otherwise the model is the simplest available and matches the ruling's own premise:** the
sidebar's visibility is now entirely the user's answer to *"is the tree useful to me right now?"*,
in every mode, with nothing else writing to it. One question, one answer, one writer.

**The tempting fix, to refuse: making `sidebarHiddenByUser` per-mode.** It would restore the
zero-toggle workflow — hidden in reading, shown in split, each remembered. Reject it. It puts the
two questions back into one piece of state, which is exactly the coupling that produced the
original bug, and it would make the sidebar's state depend on a per-tab property again. If the
zero-toggle workflow is wanted back, the honest form is a *separate* product decision about
per-mode layout memory, not a quiet re-coupling of this one.

---

## 4. Not mine, but it interacts: the 14-inch overflow just gained a state

The horizontal overflow at 14-inch with the sidebar open is being diagnosed separately. Worth
knowing before it is dispatched:

**reading mode was previously the one state that could not reach it**, because the sidebar was
force-hidden there. After this change reading mode *always* has the sidebar open, so the overflow
becomes reachable from one more mode — and reading mode is the one with 5rem of bottom padding and
a centred column, so its overflow behaviour may differ from split's rather than merely matching it.

Worth measuring in reading mode as well as split before the fix is scoped, rather than after.
