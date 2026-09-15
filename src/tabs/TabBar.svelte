<script lang="ts">
  import type { Tab } from './tabs.svelte'

  let {
    tabs,
    activePath,
    onSelect,
    onClose,
  }: {
    tabs: Tab[]
    activePath: string | null
    onSelect: (path: string) => void
    onClose: (path: string) => void
  } = $props()

  function tabTitle(tab: Tab): string | undefined {
    if (tab.detached) return `${tab.path} — removed on disk`
    if (tab.conflict) return `${tab.path} — changed on disk`
    return tab.isLoose ? tab.path : undefined
  }
</script>

<div class="tab-bar">
  {#each tabs as tab (tab.path)}
    <div class="tab" class:active={tab.path === activePath}>
      <button
        class="tab-select"
        title={tabTitle(tab)}
        onclick={() => onSelect(tab.path)}
      >
        {#if tab.isLoose}<span class="loose-marker" aria-hidden="true">◌</span>{/if}
        {tab.name}
        {#if tab.detached}<span class="status-marker" aria-hidden="true">⚠</span>{/if}
        {#if tab.conflict}<span class="status-marker" aria-hidden="true">●</span>{/if}
      </button>
      <button class="tab-close" onclick={() => onClose(tab.path)} aria-label="Close {tab.name}">
        ×
      </button>
    </div>
  {/each}
</div>

<style>
  .tab-bar {
    display: flex;
    overflow-x: auto;
    flex-shrink: 0;
    border-bottom: 1px solid var(--border, #d8d8d8);
  }

  .tab {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    border-right: 1px solid var(--border, #d8d8d8);
  }

  .tab.active {
    background: var(--border, #d8d8d8);
  }

  .tab-select {
    display: block;
    max-width: 200px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font: inherit;
    font-size: 0.85em;
    padding: 0.4em 0.4em 0.4em 0.8em;
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
  }

  .loose-marker {
    opacity: 0.6;
    margin-right: 0.3em;
  }

  .status-marker {
    margin-left: 0.3em;
    font-size: 0.8em;
    color: var(--conflict-bg, #8a5a12);
  }

  .tab-close {
    font: inherit;
    padding: 0.2em 0.7em 0.2em 0.2em;
    border: none;
    background: none;
    color: inherit;
    opacity: 0.55;
    cursor: pointer;
  }

  .tab-close:hover {
    opacity: 1;
  }
</style>
