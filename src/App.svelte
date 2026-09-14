<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'

  let message = $state('hello from the WebView')
  let reply = $state('')

  async function sendPing() {
    reply = await invoke<string>('ping', { message })
  }
</script>

<main>
  <h1>medd</h1>
  <p>Skeleton smoke test — a value crossing the IPC bridge in both directions.</p>

  <div class="row">
    <input bind:value={message} />
    <button onclick={sendPing}>Send to Rust</button>
  </div>

  {#if reply}
    <p class="reply">{reply}</p>
  {/if}
</main>

<style>
  main {
    max-width: 32rem;
    margin: 4rem auto;
    padding: 0 1.5rem;
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
  }

  .row {
    display: flex;
    gap: 0.5rem;
  }

  input {
    flex: 1;
  }

  .reply {
    font-family: ui-monospace, monospace;
  }
</style>
