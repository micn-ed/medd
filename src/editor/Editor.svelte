<script lang="ts">
  // Creates one EditorView on mount and never reconfigures it from prop changes after that —
  // CM6 is imperative and fights a reactive framework that tries to keep re-diffing it. The
  // parent is expected to force a fresh instance for a different document by keying this
  // component on document identity, e.g. `{#key path}<Editor value={content} {onChange} />`.
  //
  // The seam that matters (architecture.md §2, D-12): this component's only public surface is
  // plain text in, plain text out via `onChange`. No `Transaction`, `EditorState`, or `ViewPlugin`
  // crosses this boundary — the rest of the app never needs to know CM6 exists.
  import { onMount, onDestroy } from 'svelte'
  import { EditorState } from '@codemirror/state'
  import { EditorView } from '@codemirror/view'
  import { history } from '@codemirror/commands'
  import { search } from '@codemirror/search'
  import { editorKeymap } from './keymap'
  import { markdownTheme } from './theme'
  import { markdownSupport } from './language'

  let { value, onChange }: { value: string; onChange: (text: string) => void } = $props()

  let container: HTMLDivElement
  let view: EditorView | undefined

  onMount(() => {
    const state = EditorState.create({
      doc: value,
      extensions: [
        markdownSupport,
        history(),
        search(),
        editorKeymap,
        markdownTheme,
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            onChange(update.state.doc.toString())
          }
        }),
      ],
    })
    view = new EditorView({ state, parent: container })
    view.focus()
  })

  onDestroy(() => {
    view?.destroy()
  })
</script>

<div class="editor-host" bind:this={container}></div>

<style>
  .editor-host {
    height: 100%;
  }

  .editor-host :global(.cm-editor) {
    height: 100%;
  }
</style>
