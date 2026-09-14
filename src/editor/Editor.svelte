<script lang="ts">
  // Mounts an EditorView around an *existing* EditorState and destroys the view on unmount —
  // it does not create or own the state itself (see extensions.ts / tabs.svelte.ts). That's what
  // makes remounting safe: the parent forces a fresh EditorView per active tab via
  // `{#key activePath}`, and because the state this view is built from lives in tabs/'s
  // retention map rather than being constructed fresh here, undo history and content survive
  // the destroy/recreate cycle a tab switch causes (plan-v0.1.md increment 8).
  //
  // The seam that matters (architecture.md §2, D-12) still holds: `EditorState` is a CM6 type,
  // and this component and tabs.svelte.ts are the only two places allowed to know that — the
  // update callback baked into the state (extensions.ts) is what lets everything downstream
  // (App.svelte, the render pipeline, autosave when it lands) deal in plain text only.
  import { onMount, onDestroy } from 'svelte'
  import { EditorView } from '@codemirror/view'
  import type { EditorState } from '@codemirror/state'

  let { state }: { state: EditorState } = $props()

  let container: HTMLDivElement
  let view: EditorView | undefined

  onMount(() => {
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
