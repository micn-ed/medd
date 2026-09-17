import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen } from '@testing-library/svelte'
import Tree from './Tree.svelte'

/**
 * The CEO created a file in an open folder and the tree never showed it. The backend had emitted
 * `tree:changed` since increment 4; nothing in the frontend listened. Both ends built and tested,
 * the wire between them never connected.
 *
 * The shape of this test is QA's, and the middle assertion is the whole point. Rendering, firing,
 * and reading the tree *looks* like an end-to-end check and is not: if the underlying data never
 * changes, a successful reload and a no-op are indistinguishable — both leave the same list on
 * screen, so the test asserts the tree still shows what it showed, which is true of a working fix
 * and equally true of nothing at all. Asserting the STALE state first is what establishes that a
 * stale result would otherwise be served, and only then does the final assertion mean anything.
 */

let entries: Array<{ path: string; name: string; kind: string }> = []
const listeners = new Map<string, Set<(e: { payload: unknown }) => void>>()

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => entries),
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    const set = listeners.get(event) ?? new Set()
    set.add(handler)
    listeners.set(event, set)
    return () => set.delete(handler)
  }),
}))

const file = (name: string) => ({ path: `/w/${name}`, name, kind: 'markdown' })
const fire = (event: string) => {
  for (const h of listeners.get(event) ?? []) h({ payload: undefined })
}
const shown = () => screen.getAllByRole('button').map((b) => b.textContent?.trim())

beforeEach(() => {
  entries = [file('a.md')]
  listeners.clear()
})
afterEach(() => vi.clearAllMocks())

describe('the tree reloads when the backend says the tree changed', () => {
  it('shows a file created after it rendered, and not before', async () => {
    render(Tree, { props: { path: '/w', onOpenFile: () => {} } })
    await vi.waitFor(() => expect(shown()).toEqual(['a.md']))

    // The fixture changes underneath, as a file created by any other program would.
    entries = [file('a.md'), file('b.md')]

    // THE DISCRIMINATING ASSERTION. Nothing refreshes on its own, so a stale result really is
    // being served — without this, the assertion below cannot distinguish a reload from a no-op.
    expect(shown()).toEqual(['a.md'])

    fire('tree:changed')
    await vi.waitFor(() => expect(shown()).toEqual(['a.md', 'b.md']))
  })

  it('is listening for the event at all, under the name Rust emits', () => {
    render(Tree, { props: { path: '/w', onOpenFile: () => {} } })
    // Not a substitute for the test above — it passes with an empty handler. It exists to tell a
    // wrong event NAME apart from a broken reload, which the other test cannot distinguish.
    expect([...listeners.keys()]).toContain('tree:changed')
  })
})
