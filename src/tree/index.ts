export type EntryKind = 'directory' | 'markdown' | 'other'

export interface TreeEntry {
  name: string
  path: string
  kind: EntryKind
}

export { default as Tree } from './Tree.svelte'
