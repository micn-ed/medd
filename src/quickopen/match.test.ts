// Fuzzy scoring tests (plan-v0.1.md increment 9, W-5). Pure functions over plain data -- no
// Tauri, no Svelte -- so exhaustive coverage costs nothing here.
import { describe, expect, test } from 'vitest'
import { basenameOf, filterAndRank, scoreEntry, type QuickOpenEntry } from './match'

function entry(relativePath: string): QuickOpenEntry {
  return { path: `/workspace/${relativePath}`, relativePath }
}

describe('basenameOf', () => {
  test('returns the last path segment', () => {
    expect(basenameOf('notes/todo.md')).toBe('todo.md')
  })

  test('a path with no slash is its own basename', () => {
    expect(basenameOf('README.md')).toBe('README.md')
  })
})

describe('scoreEntry', () => {
  test('an empty query matches everything with score 0', () => {
    expect(scoreEntry('', entry('README.md'))).toBe(0)
  })

  test('returns null when the query characters are not a subsequence, in order', () => {
    expect(scoreEntry('zzz', entry('README.md'))).toBeNull()
    expect(scoreEntry('mder', entry('README.md'))).toBeNull() // right letters, wrong order
  })

  test('matches case-insensitively', () => {
    expect(scoreEntry('readme', entry('README.md'))).not.toBeNull()
    expect(scoreEntry('README', entry('readme.md'))).not.toBeNull()
  })

  test('a consecutive run scores higher than the same letters scattered', () => {
    // Both are valid subsequence matches for "abc" against different candidates of equal length.
    const consecutive = scoreEntry('abc', entry('abcxxxx.md'))
    const scattered = scoreEntry('abc', entry('axbxcxxx.md'))
    expect(consecutive).not.toBeNull()
    expect(scattered).not.toBeNull()
    expect(consecutive!).toBeGreaterThan(scattered!)
  })

  test('a basename match outranks an equally-good match against an ancestor directory', () => {
    // "arch" matches the directory name in one candidate and the filename in the other.
    const inDirectory = scoreEntry('arch', entry('architecture/notes.md'))
    const inBasename = scoreEntry('arch', entry('notes/architecture.md'))
    expect(inDirectory).not.toBeNull()
    expect(inBasename).not.toBeNull()
    expect(inBasename!).toBeGreaterThan(inDirectory!)
  })

  test('falls back to matching the full relative path when the basename alone does not match', () => {
    // "notes" appears only in the directory, not in "architecture.md".
    expect(scoreEntry('notes', entry('notes/architecture.md'))).not.toBeNull()
  })
})

describe('filterAndRank', () => {
  test('excludes entries the query does not match at all', () => {
    const entries = [entry('README.md'), entry('architecture.md')]
    const results = filterAndRank('readme', entries)
    expect(results.map((e) => e.relativePath)).toEqual(['README.md'])
  })

  test('orders best match first', () => {
    const entries = [entry('architecture/notes.md'), entry('notes/architecture.md')]
    const results = filterAndRank('arch', entries)
    // The second entry matches "arch" against its own basename; the first only matches via its
    // directory name -- both match, but the basename hit should rank first.
    expect(results[0].relativePath).toBe('notes/architecture.md')
  })

  test('ties are broken alphabetically rather than left in input order', () => {
    const entries = [entry('zebra.md'), entry('apple.md')]
    const results = filterAndRank('', entries) // empty query -- every entry scores 0, a true tie
    expect(results.map((e) => e.relativePath)).toEqual(['apple.md', 'zebra.md'])
  })

  test('an empty entry list returns an empty result, not an error', () => {
    expect(filterAndRank('anything', [])).toEqual([])
  })
})
