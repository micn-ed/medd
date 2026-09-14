// Link classification as a markdown-it renderer rule (architecture.md §11), not a click
// interceptor: the classification is computed once, at render time, and written into the DOM as
// a `data-link-kind` attribute — so a golden-file test can assert the right kind was chosen by
// reading the rendered HTML, with no click simulation and no browser involved. The four kinds
// (architecture.md's own diagram): `external`, `workspace`, `loose`, `anchor`. A relative link
// that isn't a `.md` file doesn't fit any of the four — R-2/R-6 only cover cross-document and
// external links — so it's classified `unsupported` and left inert rather than silently
// navigating the WebView somewhere undefined.
// See taskLists.ts for why the instance type is a named, aliased import rather than the default.
import type { MarkdownIt as MarkdownItInstance } from 'markdown-it'
import { isWithinWorkspace, resolveRelativePath } from './path'

export interface RenderContext {
  documentDir: string
  workspaceRoot: string | null
}

const HTTP_URL = /^https?:\/\//i
const HAS_SCHEME = /^[a-z][a-z0-9+.-]*:/i
const IS_MARKDOWN = /\.md$/i

export function linkClassification(md: MarkdownItInstance): void {
  md.renderer.rules.link_open = (tokens, idx, options, rawEnv, self) => {
    // markdown-it types `env` as its own generic `Env | undefined` so any renderer rule is
    // assignable — `renderMarkdown` always calls `md.render(source, context)`, so this cast is
    // safe for every real call, unlike the `env` type itself.
    const env = rawEnv as unknown as RenderContext
    const token = tokens[idx]
    const rawHref = token.attrGet('href')
    if (rawHref === null) return self.renderToken(tokens, idx, options)
    // href is always a string for a link_open token in practice; attrGet's `string | number` is
    // markdown-it's generic attribute-value type (some HTML attributes are numeric, href never is).
    const href = String(rawHref)

    if (HTTP_URL.test(href)) {
      token.attrSet('data-link-kind', 'external')
    } else if (href.startsWith('#')) {
      token.attrSet('data-link-kind', 'anchor')
    } else if (HAS_SCHEME.test(href)) {
      // mailto:, tel:, ftp:, etc. — not a case any requirement asks for; inert rather than an
      // unhandled navigation attempt the CSP would silently swallow anyway.
      token.attrSet('data-link-kind', 'unsupported')
    } else {
      // A local path — resolve it against the *document's own* directory (D-15), not the
      // workspace root, so a loose file's links work from wherever it actually lives.
      const resolved = resolveRelativePath(env.documentDir, href)
      if (IS_MARKDOWN.test(resolved)) {
        token.attrSet('href', resolved)
        token.attrSet(
          'data-link-kind',
          isWithinWorkspace(resolved, env.workspaceRoot) ? 'workspace' : 'loose',
        )
      } else {
        token.attrSet('data-link-kind', 'unsupported')
      }
    }

    return self.renderToken(tokens, idx, options)
  }
}
