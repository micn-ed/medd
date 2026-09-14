// The curated highlight.js language subset (ADR-001, plan-v0.1.md §5) — a named constant so
// growing it later is a one-line change in one place, not a decision made ad hoc at a call site.
// Imports from `highlight.js/lib/core` plus individual language modules, not the full
// `highlight.js` package, which ships ~190 grammars; that's the whole point of "curated."
import hljs from 'highlight.js/lib/core'
import bash from 'highlight.js/lib/languages/bash'
import rust from 'highlight.js/lib/languages/rust'
import python from 'highlight.js/lib/languages/python'
import javascript from 'highlight.js/lib/languages/javascript'
import typescript from 'highlight.js/lib/languages/typescript'
import json from 'highlight.js/lib/languages/json'
import yaml from 'highlight.js/lib/languages/yaml'
import ini from 'highlight.js/lib/languages/ini'
import sql from 'highlight.js/lib/languages/sql'
import markdown from 'highlight.js/lib/languages/markdown'

hljs.registerLanguage('bash', bash)
hljs.registerLanguage('rust', rust)
hljs.registerLanguage('python', python)
hljs.registerLanguage('javascript', javascript)
hljs.registerLanguage('typescript', typescript)
hljs.registerLanguage('json', json)
hljs.registerLanguage('yaml', yaml)
// highlight.js has no dedicated TOML grammar; `ini`'s module declares `toml` as an alias, which
// is the standard way highlight.js consumers cover TOML. Confirmed empirically that requesting
// the language by name `toml` resolves through the alias correctly.
hljs.registerLanguage('ini', ini)
hljs.registerLanguage('sql', sql)
hljs.registerLanguage('markdown', markdown)

export { hljs }

/**
 * Highlights `code` as `lang` if it's in the curated subset (or an alias of one, e.g. `toml`);
 * otherwise returns `''` so markdown-it falls back to its own escaped, unhighlighted rendering.
 * Deliberately doesn't fall back to `highlightAuto` for an unrecognised language: guessing among
 * a subset this narrow is more likely to mislabel a block than to help — a Kotlin snippet
 * mislabelled as Rust is a worse outcome than a Kotlin snippet with no colour at all.
 */
export function highlightFencedCode(code: string, lang: string): string {
  if (!lang || !hljs.getLanguage(lang)) return ''
  return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value
}
