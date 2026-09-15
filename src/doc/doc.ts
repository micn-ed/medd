// Dirty tracking, autosave debounce, conflict state machine (architecture.md §2, plan-v0.1.md
// increment 7). Deliberately depends on tabs/ in one direction only — tabs.svelte.ts exposes a
// registration hook (`setOnDocChanged`) instead of importing this module itself, so there's no
// circular import between "the thing that owns tab/EditorState identity" and "the thing that
// decides when to save it".
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  applyExternalContent,
  getTab,
  markConflict,
  markDetached,
  markSynced,
  resolveConflictKeepMine,
  setOnDocChanged,
} from '../tabs'

const AUTOSAVE_DEBOUNCE_MS = 1000

const debounceTimers = new Map<string, ReturnType<typeof setTimeout>>()
// Guards against two overlapping document_write calls for the same path racing each other — if
// a debounce tick fires while a previous write is still in flight, using the same (not-yet-
// updated) expectedHash for a second call would make the CAS reject it as a "conflict" that is
// really just medd's own write finishing late, not a genuine external change. `pendingRetry`
// means "try again once the in-flight one settles", so a keystroke that happens not to be
// followed by any later one still eventually gets saved.
const inFlight = new Set<string>()
const pendingRetry = new Set<string>()

interface ConflictErrorPayload {
  kind: 'Conflict'
  currentContent: string
  hash: string
}

function isConflictError(e: unknown): e is ConflictErrorPayload {
  return typeof e === 'object' && e !== null && (e as { kind?: unknown }).kind === 'Conflict'
}

function scheduleAutosave(path: string): void {
  const existing = debounceTimers.get(path)
  if (existing) clearTimeout(existing)
  debounceTimers.set(
    path,
    setTimeout(() => {
      debounceTimers.delete(path)
      void performAutosave(path)
    }, AUTOSAVE_DEBOUNCE_MS),
  )
}

async function performAutosave(path: string): Promise<void> {
  const tab = getTab(path)
  if (!tab) return
  if (tab.conflict || tab.detached) return // suspended (D-11)
  if (tab.currentText === tab.lastSyncedText) return // not dirty — nothing to write

  if (inFlight.has(path)) {
    pendingRetry.add(path)
    return
  }

  inFlight.add(path)
  const textToWrite = tab.currentText
  try {
    const newHash = await invoke<string>('document_write', {
      path,
      content: textToWrite,
      expectedHash: tab.expectedHash,
    })
    markSynced(path, textToWrite, newHash)
  } catch (e) {
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
  } finally {
    inFlight.delete(path)
    if (pendingRetry.delete(path)) {
      void performAutosave(path)
    }
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

/** The conflict banner's "Keep mine": see tabs.svelte.ts's resolveConflictKeepMine for why this
 * doesn't write immediately. */
export function keepMine(path: string): void {
  resolveConflictKeepMine(path)
}
