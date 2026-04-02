<script lang="ts">
  import { onMount } from 'svelte';
  import { ws, ActiveTab, zoneId } from '../lib/stores';
  import TransportBar from './TransportBar.svelte';
  import NowPlaying from './NowPlaying.svelte';
  import Library from './Library.svelte';
  import Queue from './Queue.svelte';
  import Search from './Search.svelte';

  const activeTab = new ActiveTab();
  let tab = $derived(activeTab.value);
  let connected = $derived(ws.connected);

  onMount(() => {
    ws.connect();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) return;

      if (e.key === ' ') {
        e.preventDefault();
        const zone = ws.zones.find(z => z.id === zoneId);
        if (zone) {
          if (zone.status === 'playing') {
            ws.sendCommand({ cmd: 'pause', zone_id: zoneId });
          } else {
            ws.sendCommand({ cmd: 'play', zone_id: zoneId });
          }
        }
      } else if (e.key === '/') {
        e.preventDefault();
        activeTab.value = 'search';
        setTimeout(() => document.querySelector('input')?.focus(), 10);
      } else if (e.key === '1') activeTab.value = 'now-playing';
      else if (e.key === '2') activeTab.value = 'library';
      else if (e.key === '3') activeTab.value = 'queue';
      else if (e.key === '4') activeTab.value = 'search';
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  });
</script>

<div class="app-layout">
  <nav class="sidebar">
    <div class="logo">Kanade</div>
    
    <div class="status" class:connected>
      <div class="dot"></div>
      {connected ? 'Connected' : 'Connecting...'}
    </div>

    <button class:active={tab === 'now-playing'} onclick={() => activeTab.value = 'now-playing'}>
      Now Playing
    </button>
    <button class:active={tab === 'library'} onclick={() => activeTab.value = 'library'}>
      Library
    </button>
    <button class:active={tab === 'queue'} onclick={() => activeTab.value = 'queue'}>
      Queue
    </button>
    <button class:active={tab === 'search'} onclick={() => activeTab.value = 'search'}>
      Search
    </button>
  </nav>

  <main class="content">
    {#if tab === 'now-playing'}
      <NowPlaying />
    {:else if tab === 'library'}
      <Library />
    {:else if tab === 'queue'}
      <Queue />
    {:else if tab === 'search'}
      <Search />
    {/if}
  </main>

  <div class="transport">
    <TransportBar />
  </div>
</div>

<style>
  .app-layout {
    display: grid;
    grid-template-columns: 200px 1fr;
    grid-template-rows: 1fr 80px;
    height: 100vh;
    width: 100vw;
  }

  .sidebar {
    background-color: var(--bg-dark);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-right: 1px solid var(--bg-highlight);
  }

  .logo {
    font-size: 24px;
    font-weight: bold;
    color: var(--accent);
    margin-bottom: 20px;
    padding-left: 12px;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--comment);
    padding-left: 12px;
    margin-bottom: 12px;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: var(--red);
  }

  .status.connected .dot {
    background-color: var(--green);
  }

  button {
    text-align: left;
    padding: 10px 12px;
    border-radius: 6px;
    color: var(--fg-dark);
    transition: all 0.2s;
  }

  button:hover {
    background-color: var(--bg-highlight);
    color: var(--fg);
  }

  button.active {
    background-color: var(--accent);
    color: var(--bg);
    font-weight: 500;
  }

  .content {
    background-color: var(--bg);
    overflow: hidden;
    position: relative;
  }

  .transport {
    grid-column: 1 / -1;
    background-color: var(--bg-dark);
    border-top: 1px solid var(--bg-highlight);
    z-index: 10;
  }
</style>
