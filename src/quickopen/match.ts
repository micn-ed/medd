// Fuzzy filename scoring for quick-open (Cmd+P, plan-v0.1.md increment 9, W-5). Deliberately
// simple, per the plan: substring-with-gaps (every query character must appear in the candidate,
// in order, but not necessarily adjacent), with a bonus for runs of consecutive matches and for
// matching the file's own basename rather than only some ancestor directory in its path. This is
// plenty for a workspace's worth of filenames and is not meant to grow into a real fuzzy matcher.

export interface QuickOpenEntry {
  path: string
  relativePath: string
}

const BASENAME_MATCH_BONUS = 100

export function basenameOf(relativePath: string): string {
  const idx = relativePath.lastIndexOf('/')
  return idx === -1 ? relativePath : relativePath.slice(idx + 1)
}

// Null when `query`'s characters don't all appear in `candidate`, in order. Otherwise a score
// that rewards consecutive runs: each matched character scores 1 plus its current run length, so
// "abc" matching contiguously scores 1+2+3=6 while the same three letters scattered apart score
// 1+1+1=3.
function subsequenceScore(query: string, candidate: string): number | null {
  let qi = 0
  let score = 0
  let run = 0
  for (let ci = 0; ci < candidate.length && qi < query.length; ci++) {
    if (candidate[ci] === query[qi]) {
      run += 1
      score += run
      qi += 1
    } else {
      run = 0
    }
  }
  return qi === query.length ? score : null
}

// Null when the query doesn't match at all. Prefers a match against the basename (a query typed
// to find "readme.md" shouldn't lose to some unrelated file that merely lives in a directory
// named similarly) — scored against the basename first, and only falls back to the full relative
// path when the basename doesn't match.
export function scoreEntry(query: string, entry: QuickOpenEntry): number | null {
  if (query === '') return 0

  const q = query.toLowerCase()
  const basename = basenameOf(entry.relativePath).toLowerCase()
  const basenameScore = subsequenceScore(q, basename)
  if (basenameScore !== null) return basenameScore + BASENAME_MATCH_BONUS

  return subsequenceScore(q, entry.relativePath.toLowerCase())
}

// Filters to entries the query matches at all, ranked best-first; ties broken alphabetically so
// the result order is stable rather than depending on the input's original order.
export function filterAndRank(query: string, entries: QuickOpenEntry[]): QuickOpenEntry[] {
  const scored: { entry: QuickOpenEntry; score: number }[] = []
  for (const entry of entries) {
    const score = scoreEntry(query, entry)
    if (score !== null) scored.push({ entry, score })
  }
  scored.sort(
    (a, b) => b.score - a.score || a.entry.relativePath.localeCompare(b.entry.relativePath),
  )
  return scored.map((s) => s.entry)
}
