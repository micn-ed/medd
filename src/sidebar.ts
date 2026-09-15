// Sidebar visibility versus per-tab reading mode (docs/ruling-sidebar-visibility.md). Two stored
// inputs answer two different questions -- "is the file tree useful to me right now?" (W-3,
// workspace-level, the toggle's own answer) and "should this document be distraction-free?"
// (E-3/E-4, per-tab) -- and this derives the one presentational fact both of them determine.
//
// Pulled out of App.svelte on purpose, not just for testability: `visible` and `showToggle` used
// to be two separate `$derived` expressions that both happened to repeat
// `workspaceRoot !== null && viewMode !== 'reading'`, with nothing but whoever next edited either
// one keeping them in agreement. Defining `visible` *in terms of* `showToggle` here makes that
// coupling structural — the exact shape of bug this file exists to fix, one level up: add a
// fourth condition to visibility later without touching this function, and the toggle silently
// goes inert in that mode again.
import type { ViewMode } from './tabs'

export interface SidebarLayout {
  visible: boolean
  showToggle: boolean
}

export function sidebarLayout(
  workspaceRoot: string | null,
  hiddenByUser: boolean,
  viewMode: ViewMode | undefined,
): SidebarLayout {
  // `viewMode` is `undefined` when a workspace is open but no tab has been clicked yet (the
  // moment right after "Open Folder…", and so the first thing a user would notice) -- pinned
  // explicitly as "not reading" rather than left to fall out of `viewMode !== 'reading'` being
  // true for `undefined` by coincidence of the comparison.
  const inReadingMode = viewMode === 'reading'
  const showToggle = workspaceRoot !== null && !inReadingMode
  return {
    visible: showToggle && !hiddenByUser,
    showToggle,
  }
}
