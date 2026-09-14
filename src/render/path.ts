// Pure POSIX path algebra for resolving relative links and images against the *referencing*
// document's own directory (R-4, D-15) — not against Node's `path` module, which isn't available
// in the WebView, and not against a browser-supplied path library, since medd is macOS-only and
// the actual need is small enough that hand-rolling it is clearer than taking a dependency for it.

/** Resolves `relative` against `baseDir`, handling `.`, `..`, and an already-absolute path. */
export function resolveRelativePath(baseDir: string, relative: string): string {
  if (relative.startsWith('/')) {
    return normalize(relative)
  }
  return normalize(`${baseDir}/${relative}`)
}

function normalize(path: string): string {
  const isAbsolute = path.startsWith('/')
  const stack: string[] = []
  for (const part of path.split('/')) {
    if (part === '' || part === '.') continue
    if (part === '..') {
      stack.pop()
    } else {
      stack.push(part)
    }
  }
  return (isAbsolute ? '/' : '') + stack.join('/')
}

/** Whether `path` is the workspace root or lives under it — the classification boundary for
 * "workspace" vs "loose" links (architecture.md §11). This is a UX classification only: it
 * decides how a link is labelled and which tab-opening affordance applies, never what
 * `document_read` is allowed to read. The actual security boundary lives in Rust
 * (workspace.rs's `classify`/`dir_list` scoping), which canonicalises against the real
 * filesystem; this is plain string comparison against a path the frontend was already given. */
export function isWithinWorkspace(path: string, workspaceRoot: string | null): boolean {
  if (!workspaceRoot) return false
  if (path === workspaceRoot) return true
  const prefix = workspaceRoot.endsWith('/') ? workspaceRoot : `${workspaceRoot}/`
  return path.startsWith(prefix)
}

/** The directory containing `path` (POSIX-only, matching medd's macOS-only scope). */
export function dirname(path: string): string {
  const idx = path.lastIndexOf('/')
  if (idx <= 0) return '/'
  return path.slice(0, idx)
}

/** The final path segment. */
export function basename(path: string): string {
  const idx = path.lastIndexOf('/')
  return idx < 0 ? path : path.slice(idx + 1)
}
