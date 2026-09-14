// Tab model tests (plan-v0.1.md increment 8). Module-level state is reset before each test since
// tabs.svelte.ts is a shared store, not a fresh instance per import.
import { beforeEach, describe, expect, test } from 'vitest'
import { EditorView } from '@codemirror/view'
import { undo } from '@codemirror/commands'
import {
  allTabs,
  activeTab,
  activeTabPath,
  closeAllTabs,
  closeTab,
  editorStateFor,
  openTab,
  setActiveTab,
  setActiveTabViewMode,
} from './tabs.svelte'

beforeEach(() => {
  closeAllTabs()
})

describe('opening tabs', () => {
  test('opening two different paths creates two tabs, the second becomes active', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    openTab('/workspace/b.md', 'b', '/workspace')

    expect(allTabs().map((t) => t.path)).toEqual(['/workspace/a.md', '/workspace/b.md'])
    expect(activeTabPath()).toBe('/workspace/b.md')
  })

  test('reopening an already-open path switches to it without resetting or duplicating', () => {
    openTab('/workspace/a.md', 'original content', '/workspace')
    openTab('/workspace/b.md', 'b', '/workspace')

    // Simulate an in-progress edit on tab a before it's revisited.
    const view = new EditorView({ state: editorStateFor('/workspace/a.md') })
    view.dispatch({ changes: { from: 0, insert: 'EDITED ' } })
    view.destroy()

    openTab('/workspace/a.md', 'original content', '/workspace')

    expect(allTabs()).toHaveLength(2)
    expect(activeTabPath()).toBe('/workspace/a.md')
    expect(editorStateFor('/workspace/a.md').doc.toString()).toBe('EDITED original content')
  })

  test('a workspace-relative path is named by its basename', () => {
    openTab('/workspace/notes/todo.md', 'x', '/workspace')
    expect(activeTab()?.name).toBe('todo.md')
    expect(activeTab()?.isLoose).toBe(false)
  })

  test('a loose path (D-15) is named by its parent directory plus basename', () => {
    openTab('/Users/me/Downloads/report.md', 'x', '/workspace')
    expect(activeTab()?.name).toBe('Downloads/report.md')
    expect(activeTab()?.isLoose).toBe(true)
  })

  test('a path is loose when no workspace is open at all', () => {
    openTab('/Users/me/notes.md', 'x', null)
    expect(activeTab()?.isLoose).toBe(true)
  })
})

describe('closing tabs (P-2: always safe, never prompts)', () => {
  test('closing a tab removes it and frees its retained state', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    closeTab('/workspace/a.md')

    expect(allTabs()).toHaveLength(0)
    expect(() => editorStateFor('/workspace/a.md')).toThrow()
  })

  test('closing the active tab falls back to the next remaining tab', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    openTab('/workspace/b.md', 'b', '/workspace')
    openTab('/workspace/c.md', 'c', '/workspace')
    setActiveTab('/workspace/b.md')

    closeTab('/workspace/b.md')

    expect(activeTabPath()).toBe('/workspace/c.md')
  })

  test('closing the last tab falls back to the previous one', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    openTab('/workspace/b.md', 'b', '/workspace')

    closeTab('/workspace/b.md')

    expect(activeTabPath()).toBe('/workspace/a.md')
  })

  test('closing the only tab leaves nothing active', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    closeTab('/workspace/a.md')
    expect(activeTabPath()).toBeNull()
  })

  test('closing an inactive tab does not disturb the active one', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    openTab('/workspace/b.md', 'b', '/workspace')
    closeTab('/workspace/a.md')
    expect(activeTabPath()).toBe('/workspace/b.md')
  })
})

describe('per-tab view mode', () => {
  test('defaults to split and only affects the active tab', () => {
    openTab('/workspace/a.md', 'a', '/workspace')
    openTab('/workspace/b.md', 'b', '/workspace')
    expect(activeTab()?.viewMode).toBe('split')

    setActiveTabViewMode('reading')
    setActiveTab('/workspace/a.md')

    expect(activeTab()?.viewMode).toBe('split')
    expect(allTabs().find((t) => t.path === '/workspace/b.md')?.viewMode).toBe('reading')
  })
})

describe('undo history survives a tab switch (the point of retaining EditorState)', () => {
  test('editing, switching away, and switching back preserves both content and undo', () => {
    openTab('/workspace/a.md', 'original', '/workspace')

    // A real EditorView, mounted against the tab's retained state — this is what "switching to
    // this tab" looks like in the app. Typing goes through the view's dispatch cycle, which is
    // what actually fires the updateListener baked into the state (a bare state.update() call
    // with no attached view would not exercise that path).
    let view = new EditorView({ state: editorStateFor('/workspace/a.md') })
    view.dispatch({ changes: { from: view.state.doc.length, insert: ' EDITED' } })
    expect(view.state.doc.toString()).toBe('original EDITED')

    // Switching away: the view is destroyed, exactly as Editor.svelte's onDestroy does.
    view.destroy()

    // Switching to another tab and back — the retained state must already reflect the edit,
    // since nothing re-reads from disk on tab switch.
    expect(editorStateFor('/workspace/a.md').doc.toString()).toBe('original EDITED')

    // Switching back: a *new* EditorView from the *same retained* state.
    view = new EditorView({ state: editorStateFor('/workspace/a.md') })
    expect(view.state.doc.toString()).toBe('original EDITED')

    // The actual claim under test: undo still works after the destroy/recreate cycle, meaning
    // history lived in the retained EditorState, not in the EditorView that got thrown away.
    undo(view)
    expect(view.state.doc.toString()).toBe('original')

    view.destroy()
  })
})
