<script lang="ts">
  import { onMount } from 'svelte';
  import { fly } from 'svelte/transition';
  import { ws, browserNode, connectBrowserNode, ActiveTab, toasts, showToast } from '../lib/stores';
  import TransportBar from './TransportBar.svelte';
  import NowPlaying from './NowPlaying.svelte';
  import Library from './Library.svelte';
  import Queue from './Queue.svelte';
  import Search from './Search.svelte';

  const activeTab = new ActiveTab();
  let tab = $derived(activeTab.value);
  let connected = $derived(ws.connected);
  let showNowPlaying = $state(false);

  onMount(() => {
    ws.connect();
    connectBrowserNode();

    const onWsToast = (e: Event) => {
      const message = (e as CustomEvent<{ message?: string }>).detail?.message;
      if (message) showToast(message);
    };

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) return;

      if (e.key === ' ') {
        e.preventDefault();
        const nid = ws.selectedNodeId;
        if (nid) {
          const node = ws.nodes.find(z => z.id === nid);
          if (node) {
            if (node.status === 'playing') {
              ws.sendCommand({ cmd: 'pause' });
            } else {
              ws.sendCommand({ cmd: 'play' });
            }
          }
        }
      } else if (e.key === '/') {
        e.preventDefault();
        activeTab.value = 'search';
        setTimeout(() => document.querySelector('input')?.focus(), 10);
      } else if (e.key === '1') activeTab.value = 'library';
      else if (e.key === '2') activeTab.value = 'queue';
      else if (e.key === '3') activeTab.value = 'search';
      else if (e.key === 'Escape' && showNowPlaying) showNowPlaying = false;
    };

    window.addEventListener('kanade-ws-toast', onWsToast);
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('kanade-ws-toast', onWsToast);
      window.removeEventListener('keydown', onKeyDown);
    };
  });
</script>

<div class="app-layout">
  <nav class="sidebar">
    <div class="logo">Kanade</div>
    
    <div class="status" class:connected>
      <div class="dot"></div>
      {connected ? 'Connected' : 'Connecting...'}
    </div>

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
    <div class="tab-panel" class:visible={tab === 'library'}><Library /></div>
    <div class="tab-panel" class:visible={tab === 'queue'}><Queue /></div>
    <div class="tab-panel" class:visible={tab === 'search'}><Search /></div>
  </main>

  <div class="transport">
    <TransportBar onOpenNowPlaying={() => showNowPlaying = true} />
  </div>

  <NowPlaying visible={showNowPlaying} onClose={() => showNowPlaying = false} />

  <nav class="bottom-tab-bar">
    <button class:active={tab === 'library'} onclick={() => activeTab.value = 'library'}>
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"/></svg>
      <span>Library</span>
    </button>
    <button class:active={tab === 'queue'} onclick={() => activeTab.value = 'queue'}>
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>
      <span>Queue</span>
    </button>
    <button class:active={tab === 'search'} onclick={() => activeTab.value = 'search'}>
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
      <span>Search</span>
    </button>
  </nav>

  <div class="toast-container">
    {#each toasts as toast (toast.id)}
      <div class="toast" transition:fly={{ y: 20, duration: 150 }}>{toast.message}</div>
    {/each}
  </div>
</div>

<style>
  .app-layout {
    display: grid;
    grid-template-columns: 200px 1fr;
    grid-template-rows: 1fr 120px;
    height: 100vh;
    height: 100dvh;
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

  .tab-panel {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    display: none;
  }

  .tab-panel.visible {
    display: block;
  }

  .transport {
    grid-column: 1 / -1;
    background-color: var(--bg-dark);
    border-top: 1px solid var(--bg-highlight);
    z-index: 10;
  }

  .toast-container {
    position: fixed;
    bottom: 100px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 100;
    pointer-events: none;
  }

  .toast {
    background-color: var(--bg-highlight);
    color: var(--fg);
    padding: 8px 20px;
    border-radius: 6px;
    font-size: 13px;
    white-space: nowrap;
  }

  .bottom-tab-bar {
    display: none;
    grid-column: 1 / -1;
    background-color: var(--bg-dark);
    border-top: 1px solid var(--bg-highlight);
    z-index: 20;
    justify-content: space-around;
    padding: 8px 0;
    padding-bottom: 12px;
  }

  .bottom-tab-bar button {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    color: var(--comment);
    background: transparent;
    border-radius: 0;
    min-width: 44px;
    min-height: 44px;
  }

  .bottom-tab-bar button:hover {
    background: transparent;
    color: var(--fg);
  }

  .bottom-tab-bar button.active {
    background: transparent;
    color: var(--accent);
    font-weight: 600;
  }

  .bottom-tab-bar span {
    font-size: 10px;
  }

  @media (max-width: 768px) {
    .app-layout {
      grid-template-columns: 1fr;
      grid-template-rows: 1fr auto auto;
    }

    .sidebar {
      display: none;
    }

    .transport {
      grid-column: 1 / -1;
      grid-row: 2;
    }

    .bottom-tab-bar {
      display: flex;
      grid-row: 3;
    }

    .toast-container {
      bottom: 150px;
    }

    @media (display-mode: standalone) {
      .app-layout {
        padding-top: env(safe-area-inset-top);
      }
    }
  }

  @media (min-width: 769px) and (max-width: 1024px) {
    .app-layout {
      grid-template-columns: 160px 1fr;
    }
    
    .sidebar {
      padding: 16px 12px;
    }
  }
</style>
