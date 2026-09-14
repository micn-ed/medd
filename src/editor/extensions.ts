// CM6 extension assembly — owned by editor/ (architecture.md §2: "CodeMirror setup, keymap,
// theme"), separate from tabs/'s job of retaining the EditorState this produces across mounts.
import { EditorState } from '@codemirror/state'
import { EditorView, type ViewUpdate } from '@codemirror/view'
import { history } from '@codemirror/commands'
import { search } from '@codemirror/search'
import { editorKeymap } from './keymap'
import { markdownTheme } from './theme'
import { markdownSupport } from './language'

/**
 * Builds a fresh EditorState for one document's initial content. Called once per tab; the
 * resulting state is retained by tabs/ across `Editor.svelte` mounts and unmounts, which is what
 * lets undo history survive a tab switch (plan-v0.1.md increment 8) — `onUpdate` is baked into
 * the state's own extensions, not re-wired per mount, so it keeps firing correctly no matter how
 * many `EditorView`s get created from this state over its lifetime.
 *
 * `onUpdate` fires on *every* update, not just document changes — tabs.svelte.ts needs every one
 * to keep its retained-state map current (including selection-only updates, so cursor position
 * survives a switch too), even though it only needs the text itself when the document changed.
 */
export function createDocumentState(
  content: string,
  onUpdate: (update: ViewUpdate) => void,
): EditorState {
  return EditorState.create({
    doc: content,
    extensions: [
      markdownSupport,
      history(),
      search(),
      editorKeymap,
      markdownTheme,
      EditorView.lineWrapping,
      EditorView.updateListener.of(onUpdate),
    ],
  })
}
