// Quick-open state module tests (plan-v0.1.md increment 9, W-5). The one behaviour worth proving
// with a test rather than reading off the code: opening the dialog is synchronous and does not
// wait on the backend scan -- that's the actual content of "a workspace scan must not block the
// dialog opening" once it's in the frontend, not just an architectural intention.
import { beforeEach, describe, expect, test, vi } from 'vitest'
import type { QuickOpenEntry } from './match'

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

const {
  isQuickOpenOpen,
  openQuickOpen,
  closeQuickOpen,
  quickOpenQuery,
  setQuickOpenQuery,
  quickOpenResults,
  quickOpenSelectedIndex,
  quickOpenLoading,
  moveQuickOpenSelection,
  quickOpenSelectedEntry,
} = await import('./quickopen.svelte')

function entry(relativePath: string): QuickOpenEntry {
  return { path: `/workspace/${relativePath}`, relativePath }
}

beforeEach(() => {
  invokeMock.mockReset()
  closeQuickOpen()
})

describe('opening the dialog', () => {
  test('flips isOpen synchronously, before the backend scan resolves', async () => {
    let resolveFetch: (entries: QuickOpenEntry[]) => void = () => {}
    invokeMock.mockReturnValue(
      new Promise<QuickOpenEntry[]>((resolve) => {
        resolveFetch = resolve
      }),
    )

    openQuickOpen()

    expect(isQuickOpenOpen()).toBe(true) // true immediately -- no await happened yet
    expect(quickOpenLoading()).toBe(true)
    expect(quickOpenResults()).toEqual([]) // nothing back from the backend yet

    resolveFetch([entry('a.md')])
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(quickOpenResults().map((e) => e.relativePath)).toEqual(['a.md'])
    expect(quickOpenLoading()).toBe(false)
  })

  test('resets the query and selection from whatever a previous session left behind', async () => {
    invokeMock.mockResolvedValue([entry('a.md'), entry('b.md')])
    openQuickOpen()
    await Promise.resolve()
    await Promise.resolve()
    setQuickOpenQuery('a')
    moveQuickOpenSelection(1)
    closeQuickOpen()

    invokeMock.mockResolvedValue([entry('a.md'), entry('b.md')])
    openQuickOpen()

    expect(quickOpenQuery()).toBe('')
    expect(quickOpenSelectedIndex()).toBe(0)
  })

  test('a failed scan leaves the list empty rather than throwing', async () => {
    invokeMock.mockRejectedValue({ kind: 'Io', message: 'no workspace open' })

    openQuickOpen()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()

    expect(quickOpenResults()).toEqual([])
    expect(quickOpenLoading()).toBe(false)
  })
})

describe('query and selection', () => {
  beforeEach(async () => {
    invokeMock.mockResolvedValue([entry('a.md'), entry('bb.md'), entry('ccc.md')])
    openQuickOpen()
    await Promise.resolve()
    await Promise.resolve()
  })

  test('narrows results to what the query matches', () => {
    setQuickOpenQuery('bb')
    expect(quickOpenResults().map((e) => e.relativePath)).toEqual(['bb.md'])
  })

  test('typing resets the selection to the top result', () => {
    moveQuickOpenSelection(1)
    expect(quickOpenSelectedIndex()).toBe(1)
    setQuickOpenQuery('a')
    expect(quickOpenSelectedIndex()).toBe(0)
  })

  test('moveQuickOpenSelection wraps around at both ends', () => {
    expect(quickOpenSelectedIndex()).toBe(0)
    moveQuickOpenSelection(-1)
    expect(quickOpenSelectedIndex()).toBe(2) // wraps to the last result
    moveQuickOpenSelection(1)
    expect(quickOpenSelectedIndex()).toBe(0) // wraps back to the first
  })

  test('selectedEntry reflects the selected index', () => {
    moveQuickOpenSelection(1)
    expect(quickOpenSelectedEntry()?.relativePath).toBe('bb.md')
  })

  test('with no results, selection stays at 0 and selectedEntry is null', () => {
    setQuickOpenQuery('nothing matches this')
    expect(quickOpenSelectedIndex()).toBe(0)
    expect(quickOpenSelectedEntry()).toBeNull()
    moveQuickOpenSelection(1) // must not throw on an empty list
    expect(quickOpenSelectedIndex()).toBe(0)
  })
})

describe('closeQuickOpen', () => {
  test('closes the dialog without clearing the query', async () => {
    invokeMock.mockResolvedValue([entry('a.md')])
    openQuickOpen()
    await Promise.resolve()
    await Promise.resolve()
    setQuickOpenQuery('a')

    closeQuickOpen()

    expect(isQuickOpenOpen()).toBe(false)
  })
})
