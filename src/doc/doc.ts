// Dirty tracking, autosave debounce, conflict state machine (architecture.md §2, plan-v0.1.md
// increment 7). Deliberately depends on tabs/ in one direction only — tabs.svelte.ts exposes a
// registration hook (`setOnDocChanged`) instead of importing this module itself, so there's no
// circular import between "the thing that owns tab/EditorState identity" and "the thing that
// decides when to save it".
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  applyExternalContent,
  currentGeneration,
  getTab,
  markConflict,
  markDetached,
  markSynced,
  resolveConflictKeepMine,
  setOnDocChanged,
  setOnTabClosing,
} from '../tabs'

const AUTOSAVE_DEBOUNCE_MS = 1000

interface ConflictErrorPayload {
  kind: 'Conflict'
  currentContent: string
  hash: string
}

function isConflictError(e: unknown): e is ConflictErrorPayload {
  return typeof e === 'object' && e !== null && (e as { kind?: unknown }).kind === 'Conflict'
}

interface WriteRequest {
  content: string
  expectedHash: string
  generation: number
}

/** Everything doc.ts owes to disk for one path, kept under a single entry rather than three
 * independent path-keyed collections. That consolidation is what exposed (and fixes) a gap a
 * leader review found: a debounce tick that arrives while a previous write is still in flight for
 * the same path used to just remember "try again by re-reading the tab later" (`pendingRetry`).
 * If the tab closed before that retry ran, "later" never came — the tab's own `EditorState` was
 * gone, so there was nothing left to re-read, and a real, on-screen edit was silently dropped.
 * `queued` instead captures the write's *content* the moment the collision is detected, so it
 * needs nothing from the tab by the time it actually fires. */
interface PathState {
  timer?: ReturnType<typeof setTimeout>
  writing?: boolean
  queued?: WriteRequest
}

const pending = new Map<string, PathState>()

function stateFor(path: string): PathState {
  let state = pending.get(path)
  if (!state) {
    state = {}
    pending.set(path, state)
  }
  return state
}

/** Same class of session-lifetime growth as `generations` in tabs.svelte.ts and Rust's
 * `last_known` until `document_close` exists (architecture.md §8's drift ledger) — an entry is
 * only ever removed once there's nothing left in it, never proactively for a path that's gone
 * quiet. */
function forgetIfIdle(path: string): void {
  const state = pending.get(path)
  if (state && !state.timer && !state.writing && !state.queued) {
    pending.delete(path)
  }
}

function scheduleAutosave(path: string): void {
  const state = stateFor(path)
  if (state.timer) clearTimeout(state.timer)
  state.timer = setTimeout(() => {
    state.timer = undefined
    requestWrite(path)
  }, AUTOSAVE_DEBOUNCE_MS)
}

/** Cancels a still-pending debounce for `path` and issues the write immediately instead of
 * waiting out the rest of the window. A **primitive**, deliberately shaped to have more than one
 * caller rather than living inline inside whatever triggered it: `closeTab` and `closeAllTabs`
 * both call it today (increment-7 QA finding 1 — closing used to silently drop whatever the
 * debounce hadn't yet written), and a quit/shutdown path will need to call it too (I-2's default
 * Cmd+W-quits-the-app menu binding makes that a real route to the same loss, at the scale of every
 * open tab at once — not fixed here, flagged for whoever builds that path). Every caller must run
 * this, and let it capture what it needs from the tab, *before* the tab in question stops existing
 * in whatever sense that caller is about to make true — that's what makes the guarantee "safe to
 * close/quit" actually hold rather than merely appear to, because nothing usually happens fast
 * enough to notice otherwise. A path with no pending timer needs nothing flushed here — either
 * nothing is dirty, or a write is already in flight (with any further edit already captured in
 * `queued`, not waiting on this tab to still exist later) — so there is nothing this function
 * could add in that case. */
export function flushAutosave(path: string): void {
  const state = pending.get(path)
  if (!state?.timer) return
  clearTimeout(state.timer)
  state.timer = undefined
  requestWrite(path)
}

/** Reads the tab *now* — the one moment this is always safe to do, since every caller (a
 * debounce tick firing, or a close's flush) only ever calls this while the tab still exists — and
 * decides whether to write immediately or, if something is already in flight for this path,
 * capture the request to run once that settles. Nothing downstream of this point ever re-reads
 * the tab to decide *what* to send; only whether a settled write's outcome still applies to
 * anything (`performWrite`'s generation check) does, and that's allowed to say no. */
function requestWrite(path: string): void {
  const tab = getTab(path)
  if (!tab) return
  if (tab.conflict || tab.detached) return // suspended (D-11)
  if (tab.currentText === tab.lastSyncedText) return // not dirty — nothing to write

  const request: WriteRequest = {
    content: tab.currentText,
    expectedHash: tab.expectedHash,
    generation: currentGeneration(path),
  }

  const state = stateFor(path)
  if (state.writing) {
    state.queued = request
    return
  }

  state.writing = true
  void performWrite(path, request)
}

async function performWrite(path: string, request: WriteRequest): Promise<void> {
  let settledHash: string | undefined
  try {
    settledHash = await invoke<string>('document_write', {
      path,
      content: request.content,
      expectedHash: request.expectedHash,
    })
    // Issuing and applying are independent: the write above has already reached disk regardless
    // of what happens next. Only the bookkeeping below — telling some *tab* this succeeded — needs
    // to check whether it still describes anything real ("a write's outcome may only be applied
    // to the state that issued it", architecture.md §3; increment-7 review finding 3).
    if (currentGeneration(path) === request.generation) {
      markSynced(path, request.content, settledHash)
    }
  } catch (e) {
    if (currentGeneration(path) === request.generation) {
      if (isConflictError(e)) {
        // "A rejected CAS write becoming a conflict" (plan-v0.1.md §7) — the same banner as a
        // watcher-driven external change, just discovered by our own write losing the race
        // instead of a filesystem event arriving first.
        markConflict(path, e.currentContent, e.hash)
      } else {
        // Not part of D-11's design (Io/NotUtf8 during autosave is an edge case the plan doesn't
        // give a UI to) — surfaced for visibility rather than silently retried forever, without
        // inventing a per-tab error UI the plan doesn't ask for.
        // eslint-disable-next-line no-console
        console.error('medd: autosave failed for', path, e)
      }
    }
  } finally {
    const state = pending.get(path)
    if (state) {
      state.writing = false
      const next = state.queued
      state.queued = undefined

      if (next) {
        if (getTab(path)) {
          // The tab is still open: re-derive the request fresh from its current state rather
          // than firing the stale capture. This is what makes a rejected write correctly suspend
          // the queued one too (the catch branch above just set `tab.conflict`, so the guard at
          // the top of `requestWrite` returns without writing) and a successful one chain onto
          // its real new hash rather than the one `next` captured before that hash existed.
          requestWrite(path)
        } else if (settledHash !== undefined) {
          // The tab closed while this write was in flight (leader review: a queued write behind
          // an in-flight one must still reach disk even so). Nothing can show a conflict banner
          // for a closed tab, so this only proceeds if the write it was chained behind actually
          // succeeded — chained onto *that* write's real hash, since `next.expectedHash` was
          // necessarily stale the moment it was captured. If it was rejected instead, there is no
          // live tab left to resolve the conflict that implies, so the conservative choice is to
          // drop the queued write rather than silently overwrite whatever caused the rejection.
          state.writing = true
          void performWrite(path, { ...next, expectedHash: settledHash })
        }
      }
    }
    forgetIfIdle(path)
  }
}

interface DocumentChangedPayload {
  path: string
  content: string
  hash: string
}

interface DocumentRemovedPayload {
  path: string
}

/** Wires tabs.svelte.ts's per-keystroke hook to the autosave debounce, and starts listening for
 * the watcher's events (architecture.md §4). Call once, at app start-up. */
export function initDocSync(): void {
  setOnDocChanged(scheduleAutosave)
  setOnTabClosing(flushAutosave)

  void listen<DocumentChangedPayload>('document:changed-on-disk', (event) => {
    const { path, content, hash } = event.payload
    const tab = getTab(path)
    if (!tab) return // no longer open — nothing to reconcile

    const dirty = tab.currentText !== tab.lastSyncedText
    if (dirty) {
      markConflict(path, content, hash)
    } else {
      applyExternalContent(path, content, hash)
    }
  })

  void listen<DocumentRemovedPayload>('document:removed-on-disk', (event) => {
    markDetached(event.payload.path)
  })
}

/** The conflict banner's "Reload": take disk's content, discard the local edits that caused the
 * conflict. */
export function reload(path: string): void {
  const tab = getTab(path)
  if (!tab?.conflict) return
  applyExternalContent(path, tab.conflict.diskContent, tab.conflict.diskHash)
}

/** The conflict banner's "Keep mine": see tabs.svelte.ts's resolveConflictKeepMine for why the
 * state transition itself doesn't write immediately (it stays pure, no I/O, so there is exactly
 * one place in medd that writes a document). That leaves *something* needing to actually schedule
 * the write, or "Keep mine" only reaches disk when a debounce timer happens to already be pending
 * — which review of increment 7 found is often not the case (a rejected CAS write spends the
 * timer that discovered the conflict; a watcher event arriving after the debounce already fired
 * leaves none). Scheduling here, unconditionally, makes "resumes normal autosave" true every time
 * rather than most of the time. */
export function keepMine(path: string): void {
  resolveConflictKeepMine(path)
  scheduleAutosave(path)
}
