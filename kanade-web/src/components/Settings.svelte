<script lang="ts">
  import { browserNode, connectionSettings, ws } from '../lib/stores';

  let visible = $derived(connectionSettings.open);
  let appConnected = $derived(ws.connected);
  let outputConnected = $derived(browserNode.connected);
  let effectiveServerValue = $derived(connectionSettings.effectiveServerValue);

  function closeIfBackdrop(event: MouseEvent) {
    if (event.target === event.currentTarget) {
      connectionSettings.closePanel();
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.target instanceof HTMLInputElement && event.key !== 'Escape') {
      return;
    }

    if (visible && event.key === 'Escape') {
      connectionSettings.closePanel();
    }
  }
</script>

{#if visible}
  <div class="settings-backdrop" onclick={closeIfBackdrop} role="presentation">
    <div class="settings-panel" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <div class="header">
        <div>
          <h2 id="settings-title">Connection Settings</h2>
          <p>Set the server host for this browser.</p>
        </div>
        <button class="icon-btn" type="button" onclick={() => connectionSettings.closePanel()} aria-label="Close settings">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>

      <div class="status-grid">
        <div class="status-card">
          <span class="label">App</span>
          <span class:online={appConnected} class="value">{appConnected ? 'Connected' : 'Disconnected'}</span>
        </div>
        <div class="status-card">
          <span class="label">Browser Output</span>
          <span class:online={outputConnected} class="value">{outputConnected ? 'Connected' : 'Disconnected'}</span>
        </div>
      </div>

      <div class="field-group">
        <label for="server-url">Server URL</label>
        <input id="server-url" type="text" bind:value={connectionSettings.serverInput} placeholder="192.168.1.50:8080" spellcheck="false" autocapitalize="off" autocomplete="off" />
        <div class="hint-row">
          <span class="hint">Current: {effectiveServerValue}</span>
          {#if connectionSettings.hasServerQueryOverride}
            <span class="badge">Overridden by ?server=</span>
          {/if}
        </div>
      </div>

      <div class="resolved-grid">
        <div>
          <span class="label">WebSocket</span>
          <code>{connectionSettings.wsUrl}</code>
        </div>
      </div>

      <div class="actions">
        <button class="secondary" type="button" onclick={() => connectionSettings.clear()}>Clear Saved</button>
        <button class="secondary" type="button" onclick={() => connectionSettings.disconnect()}>Disconnect</button>
        <button class="primary" type="button" onclick={() => connectionSettings.save()}>Save</button>
      </div>
    </div>
  </div>
{/if}

<svelte:window onkeydown={handleKeydown} />

<style>
  .settings-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1100;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.65);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    padding: 24px;
  }

  .settings-panel {
    width: min(560px, 100%);
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 24px;
    border: 1px solid var(--bg-highlight);
    border-radius: 16px;
    background: linear-gradient(180deg, var(--bg-dark), var(--bg));
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.45);
  }

  .header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  h2 {
    font-size: 24px;
    color: var(--fg);
    margin-bottom: 6px;
  }

  p {
    color: var(--comment);
    font-size: 14px;
  }

  .icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 999px;
    color: var(--comment);
    background: var(--bg-highlight);
    flex-shrink: 0;
  }

  .icon-btn:hover {
    color: var(--fg);
  }

  .status-grid,
  .resolved-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  .status-card,
  .resolved-grid > div {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 16px;
    border-radius: 12px;
    background: var(--bg-highlight);
  }

  .label {
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--comment);
  }

  .value {
    font-size: 15px;
    color: var(--red);
    font-weight: 600;
  }

  .value.online {
    color: var(--green);
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  label {
    font-size: 14px;
    font-weight: 600;
    color: var(--fg);
  }

  input {
    width: 100%;
    padding: 14px 16px;
    border: 1px solid var(--bg-highlight);
    border-radius: 10px;
    background: var(--bg-dark);
    color: var(--fg);
  }

  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: 0;
    border-color: var(--accent);
  }

  .hint-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
  }

  .hint {
    font-size: 12px;
    color: var(--comment);
  }

  .badge {
    padding: 4px 8px;
    border-radius: 999px;
    background: rgba(122, 162, 247, 0.14);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
  }

  code {
    display: block;
    color: var(--fg-dark);
    font-size: 12px;
    word-break: break-all;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: 10px;
  }

  .actions button {
    min-height: 40px;
    padding: 0 16px;
    border-radius: 10px;
    font-weight: 600;
  }

  .secondary {
    background: var(--bg-highlight);
    color: var(--fg-dark);
  }

  .secondary:hover {
    color: var(--fg);
  }

  .primary {
    background: var(--accent);
    color: var(--bg);
  }

  .primary:hover {
    background: var(--accent-hover);
  }

  @media (max-width: 640px) {
    .settings-backdrop {
      padding: 12px;
      align-items: flex-end;
    }

    .settings-panel {
      width: 100%;
      max-height: min(760px, 92dvh);
      overflow-y: auto;
      border-radius: 20px 20px 0 0;
      padding-bottom: max(24px, env(safe-area-inset-bottom));
    }

    .status-grid,
    .resolved-grid {
      grid-template-columns: 1fr;
    }

    .actions button {
      min-width: calc(50% - 5px);
    }
  }
</style>
