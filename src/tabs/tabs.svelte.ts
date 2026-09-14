// Tab model and per-tab EditorState retention (architecture.md §2, plan-v0.1.md increment 8).
// A module-level Svelte 5 store: `$state` here is shared reactive state for every importer, the
// same pattern a component uses internally, just not scoped to one.
import type { EditorState } from '@codemirror/state'
import { createDocumentState } from '../editor/extensions'
import { basename, dirname, isWithinWorkspace } from '../render/path'

export type ViewMode = 'split' | 'reading' | 'source'

export interface Tab {
  path: string
  name: string
  isLoose: boolean
  viewMode: ViewMode
  currentText: string
}

// EditorState instances are CM6 class instances, not plain data, and are kept out of the
// $state-proxied tabs array deliberately: Svelte 5's $state wraps objects in a reactive Proxy,
// and a third-party class never designed to be proxied (private fields, identity-keyed internal
// maps) is exactly the kind of thing that trick can silently break. A plain Map alongside the
// reactive array is the same principle as the seam in architecture.md §2 — CM6 types stay where
// something that actually needs them can see them, and nowhere else.
const editorStates = new Map<string, EditorState>()

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

/** The retained state for `path`. Throws if `path` isn't an open tab — every caller only ever
 * asks for the active tab's state, which by construction is always present. */
export function editorStateFor(path: string): EditorState {
  const state = editorStates.get(path)
  if (!state) throw new Error(`tabs: no retained EditorState for ${path}`)
  return state
}

/**
 * Opens `path` as a tab, or switches to it if it's already open. Reopening never re-reads
 * content or resets state — there's no autosave yet to have safely persisted anything, so
 * silently discarding in-progress edits on a second click is exactly the kind of bug this guards
 * against before it can exist.
 */
export function openTab(path: string, content: string, workspaceRoot: string | null): void {
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
      }
    }),
  )

  tabs.push({ path, name, isLoose, viewMode: 'split', currentText: content })
  activePath = path
}

/** Closes a tab, freeing its retained state — no buffer cache, no recently-closed retention.
 * Always safe, never prompts (P-2): nothing here is unsaved in any sense autosave (increment 7)
 * hasn't already made safe, and keeping that true as increments land is increment 7's job, not
 * this one's. */
export function closeTab(path: string): void {
  const index = tabs.findIndex((t) => t.path === path)
  if (index === -1) return

  tabs.splice(index, 1)
  editorStates.delete(path)

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
  activePath = null
}

// A loose file isn't in any visible tree (D-15), so its tab needs enough of the path to be
// distinguishable from another loose file sharing a name — the parent directory's own name plus
// the filename, not the full path, which would crowd a tab strip out fast. The full path is
// still available as a tooltip (TabBar.svelte) for anyone who needs to be sure.
function looseLabel(path: string): string {
  return `${basename(dirname(path))}/${basename(path)}`
}
