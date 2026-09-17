<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
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

  // The backend has emitted `tree:changed` since increment 4 and nothing in the frontend ever
  // listened for it, so the tree loaded once when it appeared and never again: a file created in
  // an open folder -- by the user, by git, by anything -- stayed invisible until the folder was
  // reopened. Both ends were built and tested; the wire between them was never connected.
  //
  // Not a refresh button, though one was offered. A button is the larger change: a control, a
  // label, a place to put it, and a user who has to learn the tree can be wrong and that fixing
  // it is their job. This is the half-built thing finished.
  //
  // Per instance rather than one listener at the root: `Tree` is recursive, one component per
  // expanded directory, and each reloads only its own level. A single root listener would have to
  // remount the whole tree and would drop expansion state.
  $effect(() => {
    const pending = listen('tree:changed', () => {
      void load()
    })
    return () => {
      void pending.then((unlisten) => unlisten())
    }
  })

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
