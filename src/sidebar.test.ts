// Exhaustive over both inputs (docs/ruling-sidebar-visibility.md): a pure boolean derivation needs
// no component mounted to test completely, unlike App.svelte itself (which drags in Tauri IPC,
// tabs, doc, tree, editor and render) -- this is the test QA's "state versus what the user sees"
// finding would otherwise have left uncovered here too.
//
// It was exhaustive over *three* inputs until reading mode was taken out of the derivation by
// product decision (docs/ruling-reading-mode-sidebar.md). The `viewMode` cases are gone because
// the parameter is gone, not because they stopped mattering -- an unused parameter is where
// someone reintroduces a condition without reading the file.
import { describe, expect, test } from 'vitest'
import { sidebarLayout } from './sidebar'

const OPEN = '/workspace'
const NONE = null

describe('sidebarLayout', () => {
  test.each([
    // workspaceRoot, hiddenByUser,  expected { visible, showToggle }
    [NONE, false, { visible: false, showToggle: false }],
    [NONE, true, { visible: false, showToggle: false }],
    [OPEN, false, { visible: true, showToggle: true }],
    [OPEN, true, { visible: false, showToggle: true }],
  ] as const)('workspaceRoot=%s hiddenByUser=%s -> %o', (workspaceRoot, hiddenByUser, expected) => {
    expect(sidebarLayout(workspaceRoot, hiddenByUser)).toEqual(expected)
  })

  test('a workspace with no tab clicked yet shows the sidebar', () => {
    // The moment right after "Open Folder…", and so the first thing a user sees. This case used to
    // be pinned because `viewMode` was `undefined` here and the derivation's `!== 'reading'`
    // happened to be true for it by coincidence of the comparison rather than by intent. The
    // parameter is gone, so the coincidence is too -- but the *scenario* is unchanged and is still
    // worth asserting, because it is the state a first-time user is looking at.
    expect(sidebarLayout(OPEN, false)).toEqual({ visible: true, showToggle: true })
  })

  test('every view mode behaves identically, because none of them is an input any more', () => {
    // Reading mode keeping the sidebar is the product decision; this asserts the *mechanism* --
    // that no view mode can reach this function at all. A regression that reintroduced a mode
    // condition would have to change the signature to break this, which is the point of removing
    // the parameter rather than ignoring it.
    expect(sidebarLayout.length).toBe(2)
  })

  test('visible is never true while showToggle is false -- the coupling the derivation exists for', () => {
    for (const workspaceRoot of [NONE, OPEN]) {
      for (const hiddenByUser of [false, true]) {
        const layout = sidebarLayout(workspaceRoot, hiddenByUser)
        if (!layout.showToggle) {
          expect(layout.visible).toBe(false)
        }
      }
    }
  })
})
