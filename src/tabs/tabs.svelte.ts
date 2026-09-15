// Tab model and per-tab EditorState retention (architecture.md §2, plan-v0.1.md increment 8),
// extended in increment 7 with the per-tab sync state D-11's conflict machinery needs. A
// module-level Svelte 5 store: `$state` here is shared reactive state for every importer, the
// same pattern a component uses internally, just not scoped to one.
import type { EditorState } from '@codemirror/state'
import type { EditorView } from '@codemirror/view'
import { createDocumentState } from '../editor/extensions'
import { basename, dirname, isWithinWorkspace } from '../render/path'

export type ViewMode = 'split' | 'reading' | 'source'

export interface Conflict {
  diskContent: string
  diskHash: string
}

export interface Tab {
  path: string
  name: string
  isLoose: boolean
  viewMode: ViewMode
  currentText: string
  /** What medd last confirmed is on disk for this document — the initial read, or the content
   * as of the last successful autosave, Reload, or Keep-mine. `dirty` is always derived from
   * comparing this to `currentText`, never stored (architecture.md §3). */
  lastSyncedText: string
  /** The compare-and-swap baseline for this tab's next `document_write` (architecture.md §3). */
  expectedHash: string
  /** Set when a genuine external change arrives while this tab is dirty (D-11): autosave is
   * suspended and a banner offers Reload / Keep mine. `null` the rest of the time. */
  conflict: Conflict | null
  /** Set when the file this tab points at was removed on disk (D-11): the tab stays open holding
   * its text, autosave stays suspended, and medd never silently recreates it. */
  detached: boolean
}

// EditorState instances are CM6 class instances, not plain data, and are kept out of the
// $state-proxied tabs array deliberately: Svelte 5's $state wraps objects in a reactive Proxy,
// and a third-party class never designed to be proxied (private fields, identity-keyed internal
// maps) is exactly the kind of thing that trick can silently break. A plain Map alongside the
// reactive array is the same principle as the seam in architecture.md §2 — CM6 types stay where
// something that actually needs them can see them, and nowhere else.
const editorStates = new Map<string, EditorState>()

// The live view for whichever tab currently has one mounted (Editor.svelte registers itself on
// mount, unregisters on unmount) — at most one at a time, since only the active tab ever mounts
// an EditorView, and reading mode mounts none at all. Needed so an external-change reload can go
// *through* the live view when one exists (so the on-screen editor updates immediately) and fall
// back to updating the retained state directly when it doesn't (an inactive tab, or the active
// tab in reading mode) — updating the state without going through a mounted view would leave that
// view showing stale content, silently diverged from what's actually retained.
let mountedView: { path: string; view: EditorView } | null = null

// Registered once by doc/ at app start-up (App.svelte's job) so tabs.svelte.ts can announce a
// document change without importing doc.ts itself — doc.ts already needs to import *this*
// module (to read and mutate tab state), and the reverse import would be circular.
let onDocChanged: ((path: string) => void) | null = null

export function setOnDocChanged(fn: (path: string) => void): void {
  onDocChanged = fn
}

let tabs = $state<Tab[]>([])
let activePath = $state<string | null>(null)

export function allTabs(): Tab[] {
  return tabs
}

export function activeTabPath(): string | null {
  return activePath
}

export function activeTab(): Tab | undefined {
  return tabs.find((t) => t.path === activePath)
}

export function getTab(path: string): Tab | undefined {
  return tabs.find((t) => t.path === path)
}

/** The retained state for `path`. Throws if `path` isn't an open tab — every caller only ever
 * asks for an open tab's state, which by construction is always present. */
export function editorStateFor(path: string): EditorState {
  const state = editorStates.get(path)
  if (!state) throw new Error(`tabs: no retained EditorState for ${path}`)
  return state
}

/** Called by Editor.svelte on mount/unmount so external-change handling knows whether a live
 * view exists for a given tab right now. */
export function registerMountedView(path: string, view: EditorView): void {
  mountedView = { path, view }
}

export function unregisterMountedView(path: string): void {
  if (mountedView?.path === path) mountedView = null
}

/**
 * Opens `path` as a tab, or switches to it if it's already open. Reopening never re-reads
 * content or resets state — there's no autosave yet to have safely persisted anything, so
 * silently discarding in-progress edits on a second click is exactly the kind of bug this guards
 * against before it can exist.
 */
export function openTab(
  path: string,
  content: string,
  hash: string,
  workspaceRoot: string | null,
): void {
  if (tabs.some((t) => t.path === path)) {
    activePath = path
    return
  }

  const isLoose = !isWithinWorkspace(path, workspaceRoot)
  const name = isLoose ? looseLabel(path) : basename(path)

  editorStates.set(
    path,
    createDocumentState(content, (update) => {
      // The retained state must be replaced on *every* update, not only when the document
      // changes — CM6 states are immutable, so the map still holding the state this document
      // started with is exactly how "undo survives a tab switch" would quietly stop being true.
      editorStates.set(path, update.state)
      if (update.docChanged) {
        const tab = tabs.find((t) => t.path === path)
        if (tab) tab.currentText = update.state.doc.toString()
        onDocChanged?.(path)
      }
    }),
  )

  tabs.push({
    path,
    name,
    isLoose,
    viewMode: 'split',
    currentText: content,
    lastSyncedText: content,
    expectedHash: hash,
    conflict: null,
    detached: false,
  })
  activePath = path
}

/** Closes a tab, freeing its retained state — no buffer cache, no recently-closed retention.
 * Always safe, never prompts (P-2): nothing here is unsaved in any sense autosave hasn't already
 * made safe, and keeping that true is doc/'s job as it schedules and completes writes. */
export function closeTab(path: string): void {
  const index = tabs.findIndex((t) => t.path === path)
  if (index === -1) return

  tabs.splice(index, 1)
  editorStates.delete(path)
  unregisterMountedView(path)

  if (activePath === path) {
    const fallback = tabs[index] ?? tabs[index - 1]
    activePath = fallback?.path ?? null
  }
}

export function setActiveTab(path: string): void {
  if (tabs.some((t) => t.path === path)) activePath = path
}

export function setActiveTabViewMode(mode: ViewMode): void {
  const tab = activeTab()
  if (tab) tab.viewMode = mode
}

/** Closes every open tab — used when switching workspaces (a new tree has no relationship to
 * whatever was open before). */
export function closeAllTabs(): void {
  tabs = []
  editorStates.clear()
  mountedView = null
  activePath = null
}

/** Records a successful autosave (or, in future, any confirmed write) for `path`: `syncedText`
 * is exactly what was written, not whatever `currentText` happens to be *now* — typing can have
 * continued between the write starting and this call, and `dirty` must stay correct against
 * what's actually on disk, not against a snapshot that's already stale. */
export function markSynced(path: string, syncedText: string, hash: string): void {
  const tab = getTab(path)
  if (!tab) return
  tab.lastSyncedText = syncedText
  tab.expectedHash = hash
}

/** D-11's dirty branch: a genuine external change arrived while this tab has unsaved edits.
 * Suspends autosave (by existing, not by a separate flag — doc/ checks `conflict !== null`) and
 * surfaces Reload / Keep mine. */
export function markConflict(path: string, diskContent: string, diskHash: string): void {
  const tab = getTab(path)
  if (!tab) return
  tab.conflict = { diskContent, diskHash }
}

/** D-11's deletion branch: the tab stays open holding its text; autosave is suspended (again, by
 * this flag existing) so medd never silently recreates a file the user deleted. */
export function markDetached(path: string): void {
  const tab = getTab(path)
  if (!tab) return
  tab.detached = true
}

/**
 * D-11's clean-reload and Reload-button paths: replaces the tab's content with what's on disk,
 * preserving cursor and scroll as far as a single minimal-diff change can (a full-document
 * replace would map the cursor to nowhere meaningful for a small external edit). Goes through
 * the live `EditorView` when this tab has one mounted, so the on-screen editor updates
 * immediately; otherwise updates the retained `EditorState` directly. Either way, `currentText`,
 * `lastSyncedText`, and `expectedHash` all end up agreeing with disk, and any conflict clears.
 */
export function applyExternalContent(path: string, newContent: string, newHash: string): void {
  const state = editorStates.get(path)
  if (!state) return

  const change = computeMinimalChange(state.doc.toString(), newContent)

  if (mountedView?.path === path) {
    // Goes through the view's own dispatch, which fires the update listener baked into the
    // state — that's what keeps editorStates and currentText in sync, same as any keystroke.
    mountedView.view.dispatch({ changes: change })
  } else {
    const transaction = state.update({ changes: change })
    editorStates.set(path, transaction.state)
    const tab = getTab(path)
    if (tab) tab.currentText = transaction.state.doc.toString()
  }

  const tab = getTab(path)
  if (tab) {
    tab.lastSyncedText = newContent
    tab.expectedHash = newHash
    tab.conflict = null
  }
}

/** D-11's "Keep mine": adopts the new disk hash as the CAS baseline and resumes normal autosave
 * — deliberately does *not* write immediately, and deliberately does not touch the live buffer.
 * Setting `lastSyncedText` to the disk's content (not the user's edits) is what makes `dirty`
 * come out true again against the new baseline, so the *next* autosave tick actually writes:
 * "the user's next keystroke is what commits the decision" falls out of that comparison, not
 * from any special-cased immediate write here. */
export function resolveConflictKeepMine(path: string): void {
  const tab = getTab(path)
  if (!tab || !tab.conflict) return
  tab.lastSyncedText = tab.conflict.diskContent
  tab.expectedHash = tab.conflict.diskHash
  tab.conflict = null
}

/** A minimal single-range diff: the common prefix and suffix are trimmed, leaving just the
 * differing middle as one change. Not a full diff algorithm — deliberately so, since the only
 * goal is that CM6's own cursor-mapping-through-changes keeps the cursor sensible for a
 * realistic, localised external edit (someone fixed a typo in Neovim), not that this handles an
 * arbitrary rewrite gracefully. */
function computeMinimalChange(
  oldText: string,
  newText: string,
): { from: number; to: number; insert: string } {
  const maxCommon = Math.min(oldText.length, newText.length)
  let start = 0
  while (start < maxCommon && oldText[start] === newText[start]) start++

  let oldEnd = oldText.length
  let newEnd = newText.length
  while (oldEnd > start && newEnd > start && oldText[oldEnd - 1] === newText[newEnd - 1]) {
    oldEnd--
    newEnd--
  }

  return { from: start, to: oldEnd, insert: newText.slice(start, newEnd) }
}

// A loose file isn't in any visible tree (D-15), so its tab needs enough of the path to be
// distinguishable from another loose file sharing a name — the parent directory's own name plus
// the filename, not the full path, which would crowd a tab strip out fast. The full path is
// still available as a tooltip (TabBar.svelte) for anyone who needs to be sure.
function looseLabel(path: string): string {
  return `${basename(dirname(path))}/${basename(path)}`
}
