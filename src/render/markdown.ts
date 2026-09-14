// The render pipeline (architecture.md §11): markdown-it (GFM plugin set) -> highlight.js
// (fenced code) -> link/image rewriting -> DOMPurify, immediately before the result is trusted
// as HTML. Tables and strikethrough need no plugin — markdown-it's core has supported GFM tables
// and `~~strikethrough~~` for years; confirmed empirically before writing this comment rather
// than assumed. Footnotes do need one: `markdown-it-footnote` is the official markdown-it-org
// plugin, still receiving commits (most recently within the last few months) with only 3 open
// issues — healthy enough to depend on. Task lists are first-party; see taskLists.ts for why.
import MarkdownIt from 'markdown-it'
import footnote from 'markdown-it-footnote'
import DOMPurify from 'dompurify'
import { highlightFencedCode } from './languages'
import { taskLists } from './taskLists'
import { linkClassification, type RenderContext } from './linkClassification'
import { imageResolution } from './images'

const md = new MarkdownIt({
  // Raw HTML is part of CommonMark/GFM and medd renders it — DOMPurify immediately below is what
  // makes that safe (ADR-001: correctness against the user's own pasted HTML, not an attacker).
  html: true,
  linkify: true,
  highlight: highlightFencedCode,
})
  .use(footnote)
  .use(taskLists)
  .use(linkClassification)
  .use(imageResolution)

// DOMPurify's default allowed-URI check doesn't know about Tauri's `asset:` scheme — it only
// recognises http(s)/mailto/tel/etc. plus relative URLs — so without this, every rewritten image
// `src` gets silently stripped as an unrecognised protocol rather than rendered. Caught by the
// golden-file image tests actually asserting on the `src` value instead of just "an <img> exists".
// Extends DOMPurify's own default regex (dompurify/dist/purify.cjs.js) rather than replacing it,
// so http(s)/mailto/etc. keep working unchanged.
const ALLOWED_URI_REGEXP =
  /^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp|matrix|asset):|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i

/**
 * Renders `source` to sanitised HTML, ready for `{@html}`. `documentDir` and `workspaceRoot`
 * drive link and image resolution (both relative to the *document's* directory) and the
 * workspace/loose link classification.
 */
export function renderMarkdown(source: string, context: RenderContext): string {
  // markdown-it-footnote mutates the `env` object it's given (it stashes footnote refs/
  // definitions on it) and doesn't reset that state between calls — pass it the same object
  // across two `render()` calls and the second one can render leftover footnote markup for a
  // document that has no footnotes at all. Caught by the golden-file tests reusing a `workspace`
  // context literal across cases, exactly the shape a debounced re-render would reuse in the
  // real app. A fresh object per call is the fix, applied here so no caller has to know why.
  const html = md.render(source, { ...context })
  return DOMPurify.sanitize(html, { ALLOWED_URI_REGEXP })
}
