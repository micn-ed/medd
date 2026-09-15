// Exhaustive over the three inputs (docs/ruling-sidebar-visibility.md): a pure boolean derivation
// over three flattened conditions needs no component mounted to test completely, unlike App.svelte
// itself (which drags in Tauri IPC, tabs, doc, tree, editor and render) -- this is the test QA's
// "state versus what the user sees" finding would otherwise have left uncovered here too.
import { describe, expect, test } from 'vitest'
import { sidebarLayout } from './sidebar'

const OPEN = '/workspace'
const NONE = null

describe('sidebarLayout', () => {
  test.each([
    // workspaceRoot, hiddenByUser, viewMode,  expected { visible, showToggle }
    [NONE, false, undefined, { visible: false, showToggle: false }],
    [NONE, true, undefined, { visible: false, showToggle: false }],
    [NONE, false, 'reading', { visible: false, showToggle: false }],
    [NONE, true, 'reading', { visible: false, showToggle: false }],
    [OPEN, false, undefined, { visible: true, showToggle: true }],
    [OPEN, true, undefined, { visible: false, showToggle: true }],
    [OPEN, false, 'reading', { visible: false, showToggle: false }],
    [OPEN, true, 'reading', { visible: false, showToggle: false }],
  ] as const)(
    'workspaceRoot=%s hiddenByUser=%s viewMode=%s -> %o',
    (workspaceRoot, hiddenByUser, viewMode, expected) => {
      expect(sidebarLayout(workspaceRoot, hiddenByUser, viewMode)).toEqual(expected)
    },
  )

  test('a non-reading tab (source or split) behaves exactly like no tab at all', () => {
    const noTab = sidebarLayout(OPEN, false, undefined)
    expect(sidebarLayout(OPEN, false, 'source')).toEqual(noTab)
    expect(sidebarLayout(OPEN, false, 'split')).toEqual(noTab)
  })

  test('visible is never true while showToggle is false -- the coupling the derivation exists for', () => {
    for (const workspaceRoot of [NONE, OPEN]) {
      for (const hiddenByUser of [false, true]) {
        for (const viewMode of [undefined, 'source', 'split', 'reading'] as const) {
          const layout = sidebarLayout(workspaceRoot, hiddenByUser, viewMode)
          if (!layout.showToggle) {
            expect(layout.visible).toBe(false)
          }
        }
      }
    }
  })
})
