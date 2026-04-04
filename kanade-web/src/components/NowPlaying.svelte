<script lang="ts">
  import { onMount } from 'svelte';
  import { fly, fade } from 'svelte/transition';
  import { ws, mediaBase } from '../lib/stores';
  import { formatSampleRate, formatDuration } from '../lib/format';

  let { visible = false, onClose }: { visible: boolean; onClose: () => void } = $props();

  let node = $derived(ws.selectedNodeId ? ws.nodes.find(n => n.id === ws.selectedNodeId) : undefined);
  let currentTrack = $derived(ws.queue[ws.currentIndex ?? -1]);
  let artworkUrl = $derived(currentTrack?.album_id ? `${mediaBase}/media/art/${currentTrack.album_id}` : null);
  let artworkError = $state(false);
  $effect(() => { artworkUrl; artworkError = false; });
  let showDetails = $state(false);
  let showNodePicker = $state(false);
  
  let isPlaying = $derived(node?.status === 'playing');
  let position = $derived(node?.position_secs ?? 0);
  let duration = $derived(currentTrack?.duration_secs ?? 0);
  let volume = $derived(node?.volume ?? 100);

  function togglePlay(e: Event) {
    e.stopPropagation();
    if (!node) return;
    if (isPlaying) {
      ws.sendCommand({ cmd: 'pause' });
    } else {
      ws.sendCommand({ cmd: 'play' });
    }
  }

  function playNext(e: Event) {
    e.stopPropagation();
    ws.sendCommand({ cmd: 'next' });
  }

  function playPrev(e: Event) {
    e.stopPropagation();
    ws.sendCommand({ cmd: 'previous' });
  }

  function seek(e: MouseEvent) {
    e.stopPropagation();
    if (!duration || !node) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const percent = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const newPos = percent * duration;
    ws.sendCommand({ cmd: 'seek', position_secs: newPos });
  }

  function setVolume(e: Event) {
    e.stopPropagation();
    const input = e.target as HTMLInputElement;
    ws.sendCommand({ cmd: 'set_volume', volume: parseInt(input.value) });
  }

  function toggleShuffle(e: Event) {
    e.stopPropagation();
    ws.sendCommand({ cmd: 'set_shuffle', shuffle: !ws.shuffle });
  }

  function toggleRepeat(e: Event) {
    e.stopPropagation();
    const map: Record<string, 'off' | 'one' | 'all'> = {
      'off': 'all',
      'all': 'one',
      'one': 'off'
    };
    ws.sendCommand({ cmd: 'set_repeat', repeat: map[ws.repeat] });
  }

  function handleKeydown(e: KeyboardEvent) {
    if (visible && e.key === 'Escape') {
      onClose();
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
    return () => window.removeEventListener('keydown', handleKeydown);
  });
</script>

{#if visible && currentTrack}
  <div class="overlay-backdrop" transition:fade={{ duration: 200 }} onclick={onClose} role="presentation">
    <div class="now-playing-modal" transition:fly={{ y: '100%', duration: 300, opacity: 1 }} onclick={(e) => e.stopPropagation()} role="presentation">
      <button class="btn" onclick={onClose} aria-label="Close">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"></polyline></svg>
      </button>

      <div class="modal-content">
        <div class="info-wrapper">
          <div class="cover-art" onclick={() => showDetails = !showDetails}>
            {#if artworkUrl && !artworkError}
              <img src={artworkUrl} alt={currentTrack.album_title || 'Album'} onerror={() => (artworkError = true)} />
            {:else}
              <div class="cover-fallback">
                <svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
              </div>
            {/if}
            <div class="artwork-overlay" class:visible={showDetails}>
              <div class="overlay-inner">
                {#if currentTrack.composer}
                  <div class="overlay-row"><span class="overlay-label">Composer</span>{currentTrack.composer}</div>
                {/if}
                {#if currentTrack.genre}
                  <div class="overlay-row"><span class="overlay-label">Genre</span>{currentTrack.genre}</div>
                {/if}
                <div class="overlay-badges">
                  {#if currentTrack.format}
                    <span class="format-badge">{currentTrack.format.toUpperCase()}</span>
                  {/if}
                  {#if currentTrack.sample_rate}
                    <span class="format-badge">{formatSampleRate(currentTrack.sample_rate)}</span>
                  {/if}
                </div>
              </div>
            </div>
          </div>

          <div class="details">
            <h1 class="title" onclick={() => showDetails = !showDetails}>{currentTrack.title || currentTrack.file_path.split('/').pop()}</h1>
            <h2 class="artist">{currentTrack.artist || 'Unknown Artist'}</h2>
            {#if currentTrack.album_title}
              <h3 class="album">{currentTrack.album_title}</h3>
            {/if}
            <div class="extra">
              {#if currentTrack.composer}
                <span class="extra-item"><span class="extra-label">Composer</span>{currentTrack.composer}</span>
              {/if}
              {#if currentTrack.genre}
                <span class="extra-item"><span class="extra-label">Genre</span>{currentTrack.genre}</span>
              {/if}
              <div class="format-badges">
                {#if currentTrack.format}
                  <span class="format-badge">{currentTrack.format.toUpperCase()}</span>
                {/if}
                {#if currentTrack.sample_rate}
                  <span class="format-badge">{formatSampleRate(currentTrack.sample_rate)}</span>
                {/if}
              </div>
            </div>
          </div>
        </div>

        <div class="playback-controls">
          <div class="progress-container">
            <span class="time">{formatDuration(position)}</span>
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="progress-bar" onclick={seek}>
              <div class="progress-fill" style="width: {(duration ? (position / duration) * 100 : 0)}%"></div>
            </div>
            <span class="time">{formatDuration(duration)}</span>
          </div>

          <div class="buttons">
            <button class="icon sm" class:active={ws.shuffle} onclick={toggleShuffle}>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 4h9M1 12h9M14 2l-4 4 4 4"/><path d="M10 2l-4 4 4 4"/></svg>
            </button>
            <button class="icon" onclick={playPrev}>
              <svg width="24" height="24" viewBox="0 0 18 18" fill="currentColor"><rect x="1" y="3" width="3" height="12" rx="1"/><path d="M14 3l-10 6 10 6V3z"/></svg>
            </button>
            <button class="icon play" onclick={togglePlay}>
              {#if isPlaying}
                <svg width="40" height="40" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/></svg>
              {:else}
                <svg width="40" height="40" viewBox="0 0 24 24" fill="currentColor"><path d="M7 4l14 8-14 8V4z"/></svg>
              {/if}
            </button>
            <button class="icon" onclick={playNext}>
              <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M5 5l10 7-10 7V5z"/><rect x="17" y="5" width="4" height="14" rx="1"/></svg>
            </button>
            <button class="icon sm" class:active={ws.repeat !== 'off'} onclick={toggleRepeat}>
              {#if ws.repeat === 'one'}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 8a6 6 0 0112 0"/><path d="M13 5v3h-3"/><text x="7.5" y="11.5" text-anchor="middle" font-size="7" fill="currentColor" stroke="none" font-family="inherit">1</text></svg>
              {:else}
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 8a6 6 0 0112 0"/><path d="M13 5v3h-3"/></svg>
              {/if}
            </button>
          </div>
          
          <div class="volume-container">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><path d="M2 5.5v5h3l4 4v-13l-4 4H2z"/><path d="M11 4.5c.8.8 1.3 2 1.3 3.2s-.5 2.4-1.3 3.2" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <input type="range" min="0" max="100" value={volume} onchange={setVolume} onclick={(e) => e.stopPropagation()} />
          </div>

          {#if ws.nodes.length > 1}
            <div class="node-picker" onclick={(e) => e.stopPropagation()}>
              <button class="node-btn" onclick={() => showNodePicker = !showNodePicker}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><circle cx="6" cy="6" r="1" fill="currentColor"/><circle cx="6" cy="18" r="1" fill="currentColor"/></svg>
                <span class="node-label">{node?.name ?? '—'}</span>
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
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<svelte:window onclick={() => { showNodePicker = false; showDetails = false; }} />

<style>
  .overlay-backdrop {
    position: fixed;
    inset: 0;
    background-color: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .now-playing-modal {
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: radial-gradient(circle at 50% 0%, var(--bg-highlight), var(--bg));
    box-shadow: 0 -10px 40px rgba(0, 0, 0, 0.5);
  }

  .btn {
    position: absolute;
    top: 24px;
    right: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    background: transparent;
    color: var(--fg-dark);
    border: none;
    border-radius: 50%;
    z-index: 10;
    cursor: pointer;
    transition: color 0.2s;
  }
  .btn:hover { color: var(--fg); }

  .modal-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 100%;
    max-width: 600px;
    padding: 40px;
    gap: 32px;
  }

  .info-wrapper {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 32px;
  }

  .cover-art {
    width: 300px;
    height: 300px;
    background: linear-gradient(135deg, var(--bg-highlight), var(--bg-dark));
    border-radius: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.5);
    color: var(--accent);
    flex-shrink: 0;
    overflow: hidden;
    position: relative;
  }

  .cover-art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .cover-fallback {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .artwork-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    background: rgba(0, 0, 0, 0.8);
    opacity: 0;
    transition: opacity 0.15s;
    padding: 16px;
    border-radius: 12px;
  }

  .artwork-overlay.visible {
    opacity: 1;
  }

  .overlay-inner {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--fg);
  }

  .overlay-row {
    font-size: 14px;
  }

  .overlay-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    opacity: 0.6;
    margin-right: 8px;
  }

  .overlay-badges {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }

  .details {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    width: 100%;
  }

  .title {
    font-size: 32px;
    font-weight: 700;
    color: var(--fg);
    margin-bottom: 8px;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .artist {
    font-size: 20px;
    font-weight: 500;
    color: var(--comment);
    margin-bottom: 4px;
  }

  .album {
    font-size: 16px;
    font-weight: 400;
    color: var(--comment);
    margin-bottom: 12px;
  }

  .extra {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }

  .extra-item {
    font-size: 14px;
    color: var(--comment);
  }

  .extra-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    opacity: 0.5;
    margin-right: 8px;
  }

  .format-badges {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }

  .playback-controls {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 24px;
  }

  .progress-container {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .time {
    font-size: 13px;
    color: var(--comment);
    font-variant-numeric: tabular-nums;
    width: 40px;
    text-align: center;
  }

  .progress-bar {
    flex: 1;
    height: 6px;
    background-color: var(--bg-dark);
    border-radius: 3px;
    cursor: pointer;
    position: relative;
    overflow: hidden;
  }

  .progress-bar:hover .progress-fill {
    background-color: var(--accent-hover);
  }

  .progress-fill {
    height: 100%;
    background-color: var(--accent);
    border-radius: 3px;
    pointer-events: none;
    transition: width 0.1s linear;
  }

  .buttons {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 24px;
    width: 100%;
  }

  .icon {
    color: var(--fg-dark);
    transition: color 0.2s, transform 0.1s;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    background: transparent;
  }

  .icon:hover {
    color: var(--fg);
  }

  .icon:active {
    transform: scale(0.95);
  }

  .icon.active {
    color: var(--accent);
  }

  .icon.sm {
    width: 36px;
    height: 36px;
  }

  .icon.play {
    width: 64px;
    height: 64px;
    color: var(--fg);
    background-color: var(--accent);
    border-radius: 50%;
  }

  .icon.play:hover {
    background-color: var(--accent-hover);
    color: var(--bg);
  }

  .volume-container {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 200px;
    margin-top: 16px;
    color: var(--comment);
  }

  input[type=range] {
    flex: 1;
    accent-color: var(--accent);
  }

  .format-badge {
    background-color: rgba(0, 0, 0, 0.2);
    border: 1px solid var(--fg-gutter);
    color: var(--cyan);
    padding: 4px 10px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: bold;
    letter-spacing: 0.05em;
  }

  @media (max-width: 768px) {
    .now-playing-modal {
      border-radius: 0;
    }

    .modal-content {
      padding: 24px 20px 32px;
      gap: 24px;
      height: 100%;
      justify-content: center;
    }

    .btn {
      top: 16px;
      right: 16px;
    }

    .info-wrapper {
      width: 100%;
      align-items: center;
      padding-top: 0;
      gap: 24px;
    }

    @media (display-mode: standalone) {
      .btn {
        top: calc(env(safe-area-inset-top) + 16px);
      }

      .info-wrapper {
        padding-top: env(safe-area-inset-top);
      }
    }

    .cover-art {
      width: min(340px, 72vw);
      height: min(340px, 72vw);
      border-radius: 12px;
      box-shadow: 0 20px 40px rgba(0, 0, 0, 0.5);
      cursor: pointer;
    }

    .title {
      font-size: 24px;
      cursor: pointer;
    }

    .artist {
      font-size: 16px;
    }

    .extra {
      display: none;
    }

    .playback-controls {
      padding: 24px 0 0;
    }

    .volume-container {
      width: 240px;
    }

    .buttons {
      gap: 16px;
    }

    .node-picker {
      position: relative;
      margin-top: 4px;
    }

    .node-btn {
      display: flex;
      align-items: center;
      gap: 6px;
      background: none;
      border: none;
      color: var(--comment);
      cursor: pointer;
      padding: 6px 10px;
      border-radius: 6px;
      font-size: 13px;
    }
    .node-btn:hover { color: var(--fg); background: var(--bg-highlight); }

    .node-label {
      max-width: 120px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .node-menu {
      position: absolute;
      bottom: 100%;
      left: 50%;
      transform: translateX(-50%);
      background: var(--bg-dark);
      border: 1px solid var(--bg-highlight);
      border-radius: 8px;
      padding: 4px;
      min-width: 140px;
      z-index: 50;
      box-shadow: 0 -4px 12px rgba(0,0,0,0.3);
      display: flex;
      flex-direction: column;
      gap: 2px;
      margin-bottom: 8px;
    }

    .node-option {
      display: block;
      width: 100%;
      text-align: left;
      font-size: 13px;
      color: var(--fg-dark);
      padding: 8px 12px;
      border-radius: 6px;
      white-space: nowrap;
    }
    .node-option:hover { background: var(--bg-highlight); color: var(--fg); }
    .node-option.active { color: var(--accent); }
  }

  @media (min-width: 769px) {
    .now-playing-modal {
      width: 80%;
      max-width: 900px;
      height: 88%;
      border-radius: 16px;
    }

    .modal-content {
      max-width: none;
      width: 100%;
      height: 100%;
      padding: 40px 48px;
    }

    .info-wrapper {
      flex-direction: row;
      align-items: flex-start;
      gap: 48px;
      flex: 1;
      min-height: 0;
    }

    .cover-art {
      width: 340px;
      height: 340px;
    }

    .artwork-overlay {
      display: none;
    }

    .details {
      align-items: flex-start;
      text-align: left;
      justify-content: center;
      min-width: 0;
    }

    .title {
      font-size: 28px;
    }
  }
</style>
