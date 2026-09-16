# Reading verification-status.md as an architect

**From:** principal architect
**Question asked:** are the items listed as unverifiable unverifiable *for the reasons given*, or is any of them a design problem wearing a testing costume?
**Read against:** `992b1e2`.

**Most of it holds.** §2's instrument table is accurate, §5's two look-like-success failure modes
are both real and correctly diagnosed, §6's fraction is the honest way to state that gap, and five
of §4's six "verified by reading" items are genuinely platform-bound — the folder-picker deadlock,
the window-close ordering, the three activation states, the built-app CSP as a *mechanism*, and
WKWebView's clipboard/IME/key handling. Each of those needs a running app on macOS, and no
in-process instrument can substitute. Saying so is worth a line, because a review that finds three
things is easier to act on if it also says where it looked and found nothing.

Three items are not what they look like.

---

## 1. The CSP is the stated mitigation for a deliberate design decision, and it has never been exercised

Listed in §4 as verified by reading, with "loading a built `.app` and attempting a blocked
resource" as what would settle it. Accurate — and it understates what is resting on it.

Increment 5 accepted an **arbitrary-read command surface** on purpose. `document_read` takes any
absolute path, because D-15 says a document outside the workspace opens as a loose tab, so no path
restriction could be applied without breaking a product decision. The plan says this in as many
words, and then gives the mitigation:

> That is fine as long as only medd's own code can call it, which means the WebView must never
> execute script it did not ship, and must never be able to make an outbound request.

So the argument that an arbitrary-read surface is acceptable **rests entirely on the CSP**, and the
CSP is in the same category as reading typography — believed correct from inspection. The harness
cannot help: it is a Vite dev server and serves no policy at all.

This is not a testing gap wearing a design costume. It is the reverse — **a design decision whose
compensating control is unverified** — and that changes what it is. It is not an item on a manual
pass alongside keybindings and theme checks; it is the one check that decides whether a knowingly
accepted risk was actually mitigated.

**Recommendation: make it a gate rather than an item.** One run against a built `.app`: confirm a
`<img src="http://…">` is blocked, confirm an inline `<script>` in rendered Markdown does not
execute, confirm the asset protocol serves the workspace root and refuses a path outside it. If any
of those fails, the arbitrary-read surface is unmitigated and that is a v0.1 blocker rather than a
hardening item. Not mine to sequence — but it should be sequenced as what it is.

---

## 2. The harness cannot reach the conflict banner or the detached state — and that is the mock's doing, not Blink's

This is the costume the question was looking for.

`src/harness/eventMock.ts` discards the handler it is given and returns a no-op unlisten:

```ts
export async function listen<T>(_event, _handler): Promise<() => void> {
  return () => {}
}
```

Its own comment attributes this to the platform: *"There is no real backend here, so nothing will
ever emit `document:changed-on-disk`… External-change detection is exactly the kind of thing this
harness cannot prove."* The first half is true and the conclusion does not follow. **Firing an
event into the frontend's own listeners needs no backend at all** — it needs the mock to retain the
handlers and expose a trigger. Five lines, dev-only, in a file that is already never reachable from
a production build.

What that would buy is not a marginal improvement. It is **eye-verification of D-11's entire
user-facing surface**, which has never been looked at:

- The **conflict banner** in both themes, at a real width, with real text — currently asserted only
  as a mounted component in jsdom, and §2 says jsdom "lays nothing out".
- The **detached state**, which §3.2 records as having no visible UI beyond an `aria-hidden` glyph
  and a tooltip. That finding was reached by *reading* `App.svelte` and `TabBar.svelte`. A harness
  that could enter the state would have shown it.

The harness exists because the frontend was unverifiable by eye for six increments, and it earned
its place by catching the measure bug. **This is the same gap, still open, for the most dangerous
UI in the product** — the surface of the one decision where autosave can destroy work — and it is
recorded as a platform limitation when it is a five-line omission.

Two caveats so this is not over-read. It is still Blink, so it proves layout and appearance, not
engine behaviour. And it would not verify that Rust *emits* those events — only what the frontend
does when it receives them. Both are exactly the harness's existing scope; the point is that D-11's
UI falls inside that scope and has been excluded by accident.

---

## 3. §3.2's detached state does not violate a nice-to-have; it makes a Must false

§3.2 records the defect precisely and ends:

> The exit is closing the tab, which P-2 promises is always safe, and which takes the buffer with
> it.

That sentence is doing more work than its position suggests. **P-2 is a Must**: *"The user is never
asked 'save changes?' on close; closing a tab is safe."* In a detached tab, the only way out is the
one action the requirements guarantee is harmless, and it is not.

So this is not a defect with a workaround — **the workaround is the violation.** Which matters for
sequencing: read as "a tab gets stuck and you close it", it is an annoyance with an escape. Read as
"P-2 is false in this state", it is a requirement failure with no escape that does not lose data.

I am not re-ranking it — that is the leader's call, and the characterisation tests in
`detached-recovery.test.ts` are exactly the right way to hold it. But the document should say which
of the two it is, because the fix's priority follows from that and not from how the symptom reads.

---

## What I did not find

No instrument in §2 claims more than it establishes, which is the failure this document was written
to prevent and the one it would be most embarrassing to contain. The wire pins' "vocabulary, not
semantics" line is the sharpest self-limit in the file — they would not catch a path relative to the
wrong root — and it is exactly right.

And §5's insistence on running the whole suite per mutant, with the note that per-mutant test
selection is *"a reasonable-looking change that silently removes the thing the script is for"*, is
the single most valuable sentence here for whoever inherits it. It reads like a performance
oversight and is the opposite.
