// Image resolution against the document's own directory, not the workspace root (R-4, D-15) —
// the thing plan-v0.1.md §5 calls out as easy to get subtly wrong. A loose document's images
// live beside *it*, not beside whatever workspace happens to be open (if any).
// See taskLists.ts for why the instance type is a named, aliased import rather than the default.
import type { MarkdownIt as MarkdownItInstance } from 'markdown-it'
import { convertFileSrc } from '@tauri-apps/api/core'
import { resolveRelativePath } from './path'
import type { RenderContext } from './linkClassification'

const HTTP_URL = /^https?:\/\//i

export function imageResolution(md: MarkdownItInstance): void {
  md.renderer.rules.image = (tokens, idx, options, rawEnv, self) => {
    // See linkClassification.ts for why this cast is safe for every real call.
    const env = rawEnv as unknown as RenderContext
    const token = tokens[idx]
    const rawSrc = token.attrGet('src')
    if (rawSrc !== null) {
      const original = String(rawSrc)
      // http(s) sources are left untouched and simply won't load: the CSP's img-src doesn't
      // permit them, by design (N-6, plan-v0.1.md §5) — a broken-image icon is the correct,
      // honest signal for a local-first, offline app, not something to special-case around.
      // data: sources are also left untouched, but for the opposite reason — they carry their
      // own bytes, work fine offline, and the CSP's img-src explicitly allows `data:` for
      // exactly this case, so there's nothing to resolve or rewrite.
      if (!HTTP_URL.test(original) && !original.startsWith('data:')) {
        const resolved = resolveRelativePath(env.documentDir, original)
        token.attrSet('src', convertFileSrc(resolved))
      }
    }
    return self.renderToken(tokens, idx, options)
  }
}
