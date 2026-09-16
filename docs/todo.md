# medd — what to do next

Ready to execute. Each item carries enough context that you shouldn't need to go digging.

**Read [verification-status.md](verification-status.md) first** — particularly §1 and §2. It tells
you what the test suites do and don't establish, and several items below are "believed correct,
never observed" rather than "done".

Status was checked against `c80b619`. If you're reading this much later, re-check rather than
trust — that document explains why, and it happened to its own author.

---

## 1. Finish v0.1

### Increment 10 — CLI and single-instance routing · **built, unmerged**

On branch `medd-dev`. `route_open`, the pending-open buffer, activation, frontend launch routing,
and the shim are all written and tested. Not merged because the end-to-end verification was never
run — see §4.

Two corrections were noted and may not be applied yet:

- Extract `paths_from_argv(&[String], cwd: &Path)` and `paths_from_urls(&[Url])` as named,
  Tauri-free functions with each listener's closure reduced to one line. Without them the
  per-listener criterion has nothing to mutate.
- The criterion is **two claims**: shaping is unit-verified; hook choice is gesture-verified, and
  `RunEvent::Opened`'s gesture is deferred to v0.2 because the event cannot fire without a bundle
  declaration that is itself v0.2. See [plan-v0.1.md](plan-v0.1.md) §10.

Also unresolved: the shim hardcodes `/Applications/medd.app`. Make the bundle path overridable
(`MEDD_BUNDLE`) — better design regardless, and it removes any need to install to a system location
to test.

### Increment 11 — session state and welcome screen · **unblocked, not started**

Was unbuildable as specified until the `atomic.rs` extraction landed. `state.rs` is still a
one-line stub. Everything it needs is decided and written up in [architecture.md](architecture.md)
§7 — including four things that must be **decided rather than inherited**: creating an absent
target, the mode for a new file, validating recents on read rather than merely parsing, and the
corruption path's own failure being non-fatal.

### Increment 12 — hardening · **criteria written, nothing measured**

[plan-v0.1.md](plan-v0.1.md) §12 has pass criteria for all five measurements, derived rather than
invented. Two notes:

- The memory soak is **cycle count, not wall time** — eight hours of an idle app performs almost no
  operations and a leak is per-operation. 200 tab cycles, ~5,000 autosaves, ~5,000 watcher events,
  fit a line. It also isolates *which* operation leaks.
- The walk measurement must name a **workspace state**, not a repository. The same checkout ranges
  from ~18 to ~42,500 files depending on whether build output and dependencies are present.

---

## 2. Outstanding fixes

None are data loss. Reproductions are in [verification-status.md](verification-status.md).

| # | What | Notes |
|---|---|---|
| 1 | **Reading mode overflows its container by 104px** | Toolbar and tabs scroll off with no obvious way back. `box-sizing` plus reading mode's vertical padding, no reset anywhere. Easiest repro: click a footnote link in Reading mode — those links bypass `linkClassification` and fall through to native fragment navigation. Measured values and live causation proof in the verification doc. |
| 2 | **Every image renders with `alt=""`** | `imageResolution` replaces markdown-it's image rule and drops the step copying inline children into `alt`. One line. Do this one first: it undermines a documented rationale — the "broken-image icon is the honest signal" argument depends on the alt surviving to say what failed. |
| 3 | **Find/replace panel is unthemed** | A light slab in the dark editor. **Not** `{dark: true}` — that's static and the scheme is decided at runtime, so hardcoding inverts the problem. Drive the panel from the app's own CSS variables. |
| 4 | **`detached` is terminal and invisible** | Delete a file while open and the tab silently stops saving, with only a tab-strip glyph carrying `aria-hidden="true"`. Three closed exits, all with characterisation tests already in the suite — green, asserting the wrong behaviour deliberately, going red when fixed. |
| 5 | **Any read failure is reported as deletion** | `Err(_) => Removed`. `chmod 000` on an existing file looks deleted. Only `NotFound` should mean removed. |
| 6 | **`--error` keeps its light value in dark mode** | A value to set, not a finding. |
| 7 | **A vacuous test still sits in `doc.test.ts`** | Its coverage gap is closed by a different test; the vacuous one remains. |

Fix before v0.3, recorded not urgent: own writes still emit `tree:changed` for the containing
directory in some cases; a tracked document's deletion now emits both events.

---

## 3. v0.2 — structured for parallel work

Three streams, deliberately independent so two devs don't collide. **Use separate worktrees.**

| stream | scope | depends on |
|---|---|---|
| **Finder** | `.md` association, Open With, double-click. Unblocks `RunEvent::Opened`'s gesture verification, currently impossible. Also **drag-and-drop onto the window** — deferred here on purpose, same family, and today the gesture does nothing at all. | Increment 10 merged |
| **Neovim** | `:Medd` hands the current buffer's path to the running instance via the same entry point as the CLI. | Increment 10 merged |
| **Session restore** | Reopen with previous workspace, tabs, active file, sidebar state. | Increment 11 |

Also v0.2: relax the workspace boundary so symlinks pointing outside the root work. The predicate
and both call sites are already in one place, so it's a single edit with the agreement test still
green — which was the point of putting the answer in one predicate.

---

## 4. The one thing nobody has done

**The end-to-end quit gesture has never been run.** Launch with a file, type a character, press
Cmd+Q *inside the one-second debounce window*, check the bytes on disk.

Three of its four links are covered by tests. Only the join of the two ends is inference. It became
reachable with the CLI — the editor takes focus on mount, so a document opened by command line
needs no clicking.

**The timing is the whole test.** Without a constraint between the keystroke and the gesture, the
ordinary autosave writes the character a second later and the assertion passes against a quit flush
that does nothing. Assert the elapsed time and fail the run if it exceeds the window; run a negative
control against a build with the flush disabled and confirm it fails; assert exact bytes, not a
substring.

Two more scenarios become reachable at the same time and are cheap to take: quit with several dirty
tabs, and quit with a conflicted tab, where the file must still hold the external change.
