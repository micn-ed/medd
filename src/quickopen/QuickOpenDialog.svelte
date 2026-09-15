<script lang="ts">
  // Cmd+P quick-open dialog (plan-v0.1.md increment 9, W-5). Mounted once, unconditionally, in
  // App.svelte — this component owns the global Cmd+P binding itself (via `<svelte:window>`
  // below) rather than App.svelte wiring a separate listener that then has to know this
  // component's open/closed state, the same reasoning `doc/`'s primitives use for owning their
  // own entry points.
  import {
    isQuickOpenOpen,
    openQuickOpen,
    closeQuickOpen,
    quickOpenQuery,
    setQuickOpenQuery,
    quickOpenResults,
    quickOpenSelectedIndex,
    quickOpenLoading,
    moveQuickOpenSelection,
    quickOpenSelectedEntry,
  } from './quickopen.svelte'

  let { onOpenFile }: { onOpenFile: (path: string) => void } = $props()

  let inputEl: HTMLInputElement | undefined = $state()

  // Focus moves into the query field the moment the dialog becomes visible. `isQuickOpenOpen()`
  // flips synchronously (see quickopen.svelte.ts), but the `<input>` this targets only exists in
  // the DOM once Svelte has processed that state change into a render — an effect, rather than
  // focusing inline in `open()`, is what guarantees the element exists first.
  $effect(() => {
    if (isQuickOpenOpen()) inputEl?.focus()
  })

  function confirm(path?: string): void {
    const target = path ?? quickOpenSelectedEntry()?.path
    if (target) onOpenFile(target)
    closeQuickOpen()
  }

  function onWindowKeydown(e: KeyboardEvent): void {
    const isQuickOpenShortcut = (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'p'
    if (!isQuickOpenOpen()) {
      if (isQuickOpenShortcut) {
        e.preventDefault()
        openQuickOpen()
      }
      return
    }

    if (e.key === 'Escape') {
      e.preventDefault()
      closeQuickOpen()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      moveQuickOpenSelection(1)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      moveQuickOpenSelection(-1)
    } else if (e.key === 'Enter') {
      e.preventDefault()
      confirm()
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if isQuickOpenOpen()}
  <!-- Escape (handled globally above) is the fully keyboard-accessible way to dismiss this;
       clicking the backdrop is a mouse-only convenience layered on top of it, not a second route
       that needs its own keyboard equivalent. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={closeQuickOpen} role="presentation">
    <div
      class="palette"
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-label="Quick open"
      tabindex="-1"
    >
      <input
        bind:this={inputEl}
        class="query"
        value={quickOpenQuery()}
        oninput={(e) => setQuickOpenQuery(e.currentTarget.value)}
        placeholder="Go to file…"
        aria-label="Go to file"
        autocomplete="off"
        spellcheck="false"
      />
      <ul class="results">
        {#each quickOpenResults() as entry, i (entry.path)}
          <li>
            <button
              class="result"
              class:selected={i === quickOpenSelectedIndex()}
              onclick={() => confirm(entry.path)}
            >
              {entry.relativePath}
            </button>
          </li>
        {:else}
          <li class="status">
            {quickOpenLoading() ? 'Loading…' : 'No matching files'}
          </li>
        {/each}
      </ul>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    background: rgba(0, 0, 0, 0.3);
    z-index: 100;
  }

  .palette {
    width: min(560px, 90vw);
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background: var(--editor-bg, #fff);
    color: var(--editor-fg, #1a1a1a);
    border: 1px solid var(--border, #d8d8d8);
    border-radius: 8px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
    overflow: hidden;
  }

  .query {
    font: inherit;
    font-size: 1rem;
    padding: 0.75rem 1rem;
    border: none;
    border-bottom: 1px solid var(--border, #d8d8d8);
    background: none;
    color: inherit;
    outline: none;
  }

  .results {
    list-style: none;
    margin: 0;
    padding: 0.25rem;
    overflow-y: auto;
  }

  .status {
    padding: 0.5rem 0.75rem;
    opacity: 0.6;
    font-size: 0.9em;
  }

  .result {
    display: block;
    width: 100%;
    text-align: left;
    font: inherit;
    font-family: ui-monospace, monospace;
    font-size: 0.9em;
    padding: 0.4em 0.6em;
    border: none;
    border-radius: 4px;
    background: none;
    color: inherit;
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .result.selected {
    background: var(--border, #d8d8d8);
  }
</style>
