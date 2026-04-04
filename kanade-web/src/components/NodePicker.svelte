<script lang="ts">
  import { ws } from '../lib/stores';

  let node = $derived(ws.selectedNodeId ? ws.nodes.find(n => n.id === ws.selectedNodeId) : undefined);
  let showNodePicker = $state(false);
</script>

{#if ws.nodes.length > 1}
  <div class="node-picker" onclick={(e) => e.stopPropagation()}>
    <button class="node-btn" onclick={() => showNodePicker = !showNodePicker}>
      <span class="node-name">{node?.name ?? '—'}</span>
      <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor"><path d="M2 3l3 4 3-4z"/></svg>
    </button>
    {#if showNodePicker}
      <div class="node-menu">
        {#each ws.nodes.filter(n => n.connected) as n (n.id)}
          <button class="node-option" class:active={n.id === node?.id} onclick={() => {
            ws.sendCommand({ cmd: 'select_node', node_id: n.id });
            showNodePicker = false;
          }}>
            {n.name}
          </button>
        {/each}
      </div>
    {/if}
  </div>
{:else if node}
  <span class="node-name">{node.name}</span>
{/if}

<svelte:window onclick={() => showNodePicker = false} />

<style>
  .node-name {
    font-size: 11px;
    color: var(--comment);
    white-space: nowrap;
  }

  .node-picker {
    position: relative;
  }

  .node-menu {
    position: absolute;
    bottom: 100%;
    right: 0;
    background: var(--bg-dark);
    border: 1px solid var(--bg-highlight);
    border-radius: 6px;
    padding: 4px;
    min-width: 120px;
    z-index: 50;
    box-shadow: 0 -4px 12px rgba(0,0,0,0.3);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .node-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    color: var(--comment);
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 4px;
  }
  .node-btn:hover { color: var(--fg); background: var(--bg-highlight); }

  .node-option {
    display: block;
    width: 100%;
    text-align: left;
    font-size: 12px;
    color: var(--fg-dark);
    padding: 6px 12px;
    border-radius: 4px;
    white-space: nowrap;
  }
  .node-option:hover { background: var(--bg-highlight); color: var(--fg); }
  .node-option.active { color: var(--accent); }
</style>
