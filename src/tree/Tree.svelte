<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import type { TreeEntry } from './index'
  import Self from './Tree.svelte'

  let { path, onOpenFile }: { path: string; onOpenFile: (path: string) => void } = $props()

  let entries = $state<TreeEntry[]>([])
  let expanded = $state<Set<string>>(new Set())
  let error = $state('')

  async function load() {
    try {
      entries = await invoke<TreeEntry[]>('dir_list', { path })
    } catch (e) {
      error = String(e)
    }
  }

  load()

  function click(entry: TreeEntry) {
    if (entry.kind === 'directory') {
      const next = new Set(expanded)
      if (next.has(entry.path)) {
        next.delete(entry.path)
      } else {
        next.add(entry.path)
      }
      expanded = next
    } else if (entry.kind === 'markdown') {
      onOpenFile(entry.path)
    }
    // 'other' files are visible but inert (W-8).
  }
</script>

<ul>
  {#each entries as entry (entry.path)}
    <li>
      <button
        class="entry {entry.kind}"
        onclick={() => click(entry)}
        disabled={entry.kind === 'other'}
      >
        <span class="disclosure">
          {#if entry.kind === 'directory'}
            {expanded.has(entry.path) ? '▾' : '▸'}
          {/if}
        </span>
        {entry.name}
      </button>
      {#if entry.kind === 'directory' && expanded.has(entry.path)}
        <div class="nested">
          <Self path={entry.path} {onOpenFile} />
        </div>
      {/if}
    </li>
  {/each}
</ul>
{#if error}
  <p class="error">{error}</p>
{/if}

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .nested {
    padding-left: 1rem;
  }

  button.entry {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    padding: 0.15rem 0.25rem;
    font: inherit;
    cursor: pointer;
    color: inherit;
  }

  button.entry:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .disclosure {
    display: inline-block;
    width: 1rem;
  }

  .error {
    color: var(--error, #c0392b);
    font-size: 0.85em;
  }
</style>
