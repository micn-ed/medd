// Sidebar visibility (docs/ruling-sidebar-visibility.md, docs/ruling-reading-mode-sidebar.md).
//
// The sidebar's visibility is the user's answer to one question -- "is the file tree useful to me
// right now?" (W-3, workspace-level) -- and nothing else writes to it. It used to be two
// questions: reading mode also had an opinion, and expressed it by assigning to the stored flag,
// so one tab's per-tab setting silently overwrote a global preference and nothing ever put it
// back. The fix was to derive the presented fact from both inputs instead of letting one clobber
// the other.
//
// Reading mode has since been taken out of the derivation entirely, by product decision: it keeps
// the sidebar visible (D-4, amended). Worth recording why that is a simplification and not a
// regression -- reading mode's text never used the space it was taking. It is a 50ch centred
// column (~522px), identical with the sidebar shown or hidden at every window size medd ships in,
// and constrained only below a 762px window against a 1000px default. D-4's "collapsing the tree
// is what makes reading mode genuinely full-width" had been false since increment 6 capped the
// measure.
//
// So this is now a two-line function over two inputs, and that is the point rather than an
// embarrassment: **the function becoming trivial is what success looks like, not an argument for
// deleting it.** `visible` is defined *in terms of* `showToggle`, which is what makes the coupling
// structural instead of remembered -- inlining these four lines would put two expressions back in
// App.svelte, each repeating the other's conditions, which is the exact shape that produced the
// original bug. Adding a fourth condition to `visible` without touching `showToggle` is how a
// control that is present but does nothing becomes possible again.

export interface SidebarLayout {
  /** Whether the sidebar is on screen. */
  visible: boolean
  /** Whether the Hide/Show control is offered — true exactly when toggling it would change
   * `visible`, which is what keeps it from ever being present and inert. */
  showToggle: boolean
}

export function sidebarLayout(workspaceRoot: string | null, hiddenByUser: boolean): SidebarLayout {
  const showToggle = workspaceRoot !== null
  return {
    visible: showToggle && !hiddenByUser,
    showToggle,
  }
}
