// QA: plan-v0.1.md §7 / D-11 require the clean reload to preserve cursor and scroll.
// Nothing in the suite asserts either. Does it hold?
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { EditorView } from '@codemirror/view'
import {
  closeAllTabs, editorStateFor, openTab, registerMountedView, unregisterMountedView,
} from '../tabs'

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(), listeners: new Map<string, (e: { payload: unknown }) => void>(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((e: string, h: (x: { payload: unknown }) => void) => {
    listeners.set(e, h); return Promise.resolve(() => {})
  }),
}))
const { initDocSync } = await import('./doc')
const fire = (n: string, p: unknown) => listeners.get(n)?.({ payload: p })

beforeEach(() => {
  vi.useFakeTimers(); closeAllTabs(); invokeMock.mockReset()
  invokeMock.mockResolvedValue('h2'); listeners.clear(); initDocSync()
})
afterEach(() => vi.useRealTimers())

const DOC = ['line one', 'line two', 'line three', 'line four', 'line five'].join('\n')

describe('clean external reload preserves the cursor (D-11)', () => {
  test('an external edit ABOVE the cursor keeps the cursor on the same text', () => {
    const path = '/w/a.md'
    openTab(path, DOC, 'h1', '/w')
    const view = new EditorView({ state: editorStateFor(path) })
    registerMountedView(path, view)

    // Put the cursor just after "line four".
    const anchor = DOC.indexOf('line four') + 'line four'.length
    view.dispatch({ selection: { anchor } })
    const textBefore = view.state.doc.sliceString(0, view.state.selection.main.head)
    expect(textBefore.endsWith('line four')).toBe(true)

    // Someone in Neovim inserts a line near the top. Buffer is clean, so: silent reload.
    const updated = DOC.replace('line one', 'line one\nline one and a half')
    fire('document:changed-on-disk', { path, content: updated, hash: 'h2' })

    // Pin that the reload actually happened. Without this the test passes in a world where
    // applyExternalContent is a no-op: the cursor trivially still sits after "line four"
    // because nothing moved. Verified by mutation — this assertion is what kills that mutant.
    expect(view.state.doc.toString()).toBe(updated)

    const after = view.state.doc.sliceString(0, view.state.selection.main.head)
    expect(after.endsWith('line four')).toBe(true)

    unregisterMountedView(path); view.destroy()
  })

  test('the cursor survives a reload of an INACTIVE tab (no mounted view)', () => {
    const path = '/w/a.md'
    openTab(path, DOC, 'h1', '/w')
    // Set a selection, then unmount — as happens when the user switches tabs.
    const view = new EditorView({ state: editorStateFor(path) })
    registerMountedView(path, view)
    const anchor = DOC.indexOf('line four') + 'line four'.length
    view.dispatch({ selection: { anchor } })
    unregisterMountedView(path); view.destroy()

    const updated = DOC.replace('line one', 'line one\nline one and a half')
    fire('document:changed-on-disk', { path, content: updated, hash: 'h2' })

    const state = editorStateFor(path)
    expect(state.doc.toString()).toBe(updated)   // same reason as above
    const after = state.doc.sliceString(0, state.selection.main.head)
    expect(after.endsWith('line four')).toBe(true)
  })
})
