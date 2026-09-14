<script lang="ts">
  // Re-renders on a debounce, not per keystroke (plan-v0.1.md §5) — the parent already debounces
  // `text` upstream of this component in most cases, but this component debounces independently
  // too, since it's the one thing here that's genuinely expensive (a full markdown-it re-parse).
  import { invoke } from '@tauri-apps/api/core'
  import { renderMarkdown } from './markdown'

  let {
    text,
    documentDir,
    workspaceRoot,
    onOpenFile,
    reading = false,
  }: {
    text: string
    documentDir: string
    workspaceRoot: string | null
    onOpenFile: (path: string) => void
    reading?: boolean
  } = $props()

  const DEBOUNCE_MS = 200

  let html = $state('')
  let container: HTMLElement
  let timer: ReturnType<typeof setTimeout> | undefined

  $effect(() => {
    const source = text
    const dir = documentDir
    const root = workspaceRoot
    clearTimeout(timer)
    timer = setTimeout(() => {
      html = renderMarkdown(source, { documentDir: dir, workspaceRoot: root })
    }, DEBOUNCE_MS)
    return () => clearTimeout(timer)
  })

  function handleClick(event: MouseEvent) {
    const target = (event.target as HTMLElement).closest('a')
    if (!target || !container.contains(target)) return

    const kind = target.dataset.linkKind
    if (!kind) return

    event.preventDefault()

    switch (kind) {
      case 'external':
        invoke('open_external', { url: target.getAttribute('href') })
        break
      case 'workspace':
      case 'loose': {
        const path = target.getAttribute('href')
        if (path) onOpenFile(path)
        break
      }
      case 'anchor': {
        const id = (target.getAttribute('href') ?? '').slice(1)
        container.querySelector(`#${CSS.escape(id)}`)?.scrollIntoView({ behavior: 'smooth' })
        break
      }
      // 'unsupported' — deliberately does nothing beyond preventDefault above.
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="preview" class:reading bind:this={container} onclick={handleClick}>
  {@html html}
</div>

<style>
  .preview {
    height: 100%;
    overflow-y: auto;
  }

  /* Split view's own padding, scoped to non-reading mode via :not() rather than overridden by a
     competing .reading rule: a scoped Svelte selector (.preview.reading.svelte-hash) is *more*
     specific than preview.css's global .preview.reading, so a second declaration here would have
     silently won and zeroed out preview.css's actual reading-mode padding (2.5rem/1.5rem/5rem)
     every time — found while touching this file for increment 8, not introduced by it. */
  .preview:not(.reading) {
    padding: 0 1rem;
  }
</style>
