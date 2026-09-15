// Quick-open dialog state (Cmd+P, plan-v0.1.md increment 9, W-5).
//
// Opening the dialog (`openQuickOpen`) flips `isOpen` synchronously and only *then* kicks off the
// backend workspace scan — the scan's promise is never awaited before the dialog is considered
// open. That ordering is the actual point: plan-v0.1.md is explicit that "a workspace scan must
// not block the dialog opening", and the only way to keep that true in the frontend, not just the
// backend, is for `isOpen` to never depend on the fetch settling. `quick_open_files` is cheap on
// repeat calls (Rust-side `FileIndex` caches per workspace root, invalidated by the watcher), so
// this fetches fresh every open rather than caching client-side too — one cache, not two that can
// drift apart.
import { invoke } from '@tauri-apps/api/core'
import { filterAndRank, type QuickOpenEntry } from './match'

export type { QuickOpenEntry } from './match'

let isOpen = $state(false)
let query = $state('')
let entries = $state<QuickOpenEntry[]>([])
let selectedIndex = $state(0)
let loading = $state(false)

let results = $derived(filterAndRank(query, entries))

export function isQuickOpenOpen(): boolean {
  return isOpen
}

export function quickOpenQuery(): string {
  return query
}

export function setQuickOpenQuery(next: string): void {
  query = next
  selectedIndex = 0
}

export function quickOpenResults(): QuickOpenEntry[] {
  return results
}

export function quickOpenLoading(): boolean {
  return loading
}

// Clamped against the current results so a stale index (left over from before the backend's
// response narrowed or reordered the list) never points past the end.
export function quickOpenSelectedIndex(): number {
  if (results.length === 0) return 0
  return Math.min(selectedIndex, results.length - 1)
}

export function moveQuickOpenSelection(delta: number): void {
  if (results.length === 0) return
  const current = quickOpenSelectedIndex()
  selectedIndex = (current + delta + results.length) % results.length
}

export function quickOpenSelectedEntry(): QuickOpenEntry | null {
  return results[quickOpenSelectedIndex()] ?? null
}

export function openQuickOpen(): void {
  isOpen = true
  query = ''
  selectedIndex = 0
  loading = true

  invoke<QuickOpenEntry[]>('quick_open_files')
    .then((result) => {
      entries = result
    })
    .catch(() => {
      entries = []
    })
    .finally(() => {
      loading = false
    })
}

export function closeQuickOpen(): void {
  isOpen = false
}
