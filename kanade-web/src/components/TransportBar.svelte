<script lang="ts">
  import { ws, mediaBase } from '../lib/stores';
  import { formatDuration } from '../lib/format';

  let { onOpenNowPlaying }: { onOpenNowPlaying: () => void } = $props();

  let node = $derived(ws.nodes.find(n => n.id === ws.getNodeId()));
  let currentTrack = $derived(node?.queue[node.current_index ?? -1]);
  let artworkUrl = $derived(currentTrack?.album_id ? `${mediaBase}/media/art/${currentTrack.album_id}` : null);
  let isPlaying = $derived(node?.status === 'playing');
  let position = $derived(node?.position_secs ?? 0);
  let duration = $derived(currentTrack?.duration_secs ?? 0);
  let volume = $derived(node?.volume ?? 100);

  function togglePlay() {
    if (!node) return;
    if (isPlaying) ws.sendCommand({ cmd: 'pause', node_id: ws.getNodeId() });
    else ws.sendCommand({ cmd: 'play', node_id: ws.getNodeId() });
  }

  function playNext() { ws.sendCommand({ cmd: 'next', node_id: ws.getNodeId() }); }
  function playPrev() { ws.sendCommand({ cmd: 'previous', node_id: ws.getNodeId() }); }

  function seek(e: MouseEvent) {
    if (!duration || !node) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    ws.sendCommand({ cmd: 'seek', node_id: ws.getNodeId(), position_secs: pct * duration });
  }

  function setVolume(e: Event) {
    ws.sendCommand({ cmd: 'set_volume', node_id: ws.getNodeId(), volume: parseInt((e.target as HTMLInputElement).value) });
  }

  function adjustVolume(delta: number) {
    const v = Math.max(0, Math.min(100, volume + delta));
    ws.sendCommand({ cmd: 'set_volume', node_id: ws.getNodeId(), volume: v });
  }

  function toggleShuffle() { if (node) ws.sendCommand({ cmd: 'set_shuffle', node_id: ws.getNodeId(), shuffle: !node.shuffle }); }

  function toggleRepeat() {
    if (!node) return;
    const m: Record<string, 'off' | 'one' | 'all'> = { off: 'all', all: 'one', one: 'off' };
    ws.sendCommand({ cmd: 'set_repeat', node_id: ws.getNodeId(), repeat: m[node.repeat] });
  }
</script>

<div class="transport-bar">
  <div class="left-col">
    {#if currentTrack}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="track-info" onclick={() => onOpenNowPlaying()}>
        <img src={artworkUrl} alt="" class="artwork" onerror={(e) => (e.currentTarget.style.display = 'none')} />
        <div class="meta">
          <span class="title" title={currentTrack.title || currentTrack.file_path.split('/').pop()}>
            {currentTrack.title || currentTrack.file_path.split('/').pop()}
          </span>
          <span class="artist" title={currentTrack.artist || 'Unknown Artist'}>
            {currentTrack.artist || 'Unknown Artist'}
          </span>
        </div>
        <button class="mobile-play" onclick={(e) => { e.stopPropagation(); togglePlay(); }}>
          {#if isPlaying}
            <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="4" width="4" height="16" rx="1"/><rect x="15" y="4" width="4" height="16" rx="1"/></svg>
          {:else}
            <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M6 3l16 9-16 9V3z"/></svg>
          {/if}
        </button>
      </div>
    {/if}
  </div>

  <div class="center-col">
    <div class="controls">
      <button class="btn ic-small {node?.shuffle ? 'active' : ''}" onclick={toggleShuffle}>
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 4h9M1 12h9M14 2l-4 4 4 4"/><path d="M10 2l-4 4 4 4"/></svg>
      </button>
      <button class="btn ic-small" onclick={playPrev}>
        <svg width="18" height="18" viewBox="0 0 18 18" fill="currentColor"><rect x="1" y="3" width="3" height="12" rx="1"/><path d="M14 3l-10 6 10 6V3z"/></svg>
      </button>
      <button class="btn ic-large" onclick={togglePlay}>
        {#if isPlaying}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="4" width="4" height="16" rx="1"/><rect x="15" y="4" width="4" height="16" rx="1"/></svg>
        {:else}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M6 3l16 9-16 9V3z"/></svg>
        {/if}
      </button>
      <button class="btn ic-small" onclick={playNext}>
        <svg width="18" height="18" viewBox="0 0 18 18" fill="currentColor"><path d="M4 3l10 6-10 6V3z"/><rect x="14" y="3" width="3" height="12" rx="1"/></svg>
      </button>
      <button class="btn ic-small {node?.repeat !== 'off' ? 'active' : ''}" onclick={toggleRepeat}>
        {#if node?.repeat === 'one'}
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 8a6 6 0 0112 0"/><path d="M13 5v3h-3"/><text x="7.5" y="11.5" text-anchor="middle" font-size="7" fill="currentColor" stroke="none" font-family="inherit">1</text></svg>
        {:else}
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 8a6 6 0 0112 0"/><path d="M13 5v3h-3"/></svg>
        {/if}
      </button>
    </div>

    <div class="playback-bar">
      <span class="time">{formatDuration(position)}</span>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="progress-wrapper" onclick={seek} role="slider" aria-valuenow={position} tabindex="0">
        <div class="progress-bg">
          <div class="progress-fill" style="width: {(duration ? (position / duration) * 100 : 0)}%"></div>
        </div>
      </div>
      <span class="time">{formatDuration(duration)}</span>
    </div>
  </div>

  <div class="right-col">
    <div class="right-inner">
      <div class="vol-row">
        <button class="vol-btn" onclick={() => adjustVolume(-1)}>-</button>
        <span class="vol-label">{volume}%</span>
        <button class="vol-btn" onclick={() => adjustVolume(1)}>+</button>
      </div>
      <div class="volume-control">
        <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><path d="M2 5.5v5h3l4 4v-13l-4 4H2z"/><path d="M11 4.5c.8.8 1.3 2 1.3 3.2s-.5 2.4-1.3 3.2" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>
        <input type="range" class="volume-slider" min="0" max="100" value={volume} onchange={setVolume} />
      </div>
      {#if node}
        <span class="node-name">Node: {node.name}</span>
      {/if}
    </div>
  </div>
</div>

<style>
  .transport-bar {
    display: flex;
    align-items: stretch;
    height: 100%;
    padding: 8px 16px;
    gap: 16px;
  }

  .left-col {
    flex: 0 0 25%;
    display: flex;
    align-items: stretch;
    min-width: 0;
  }

  .track-info {
    display: flex;
    align-items: stretch;
    width: 100%;
    padding: 8px;
    border-radius: 6px;
    cursor: pointer;
    min-width: 0;
  }
  .track-info:hover { background-color: var(--bg-highlight); }

  .artwork {
    height: 100%;
    aspect-ratio: 1;
    border-radius: 4px;
    flex-shrink: 0;
    object-fit: cover;
  }

  .meta {
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-width: 0;
    padding-left: 12px;
  }

  .title {
    font-size: 13px;
    font-weight: bold;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .artist {
    font-size: 12px;
    color: var(--comment);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mobile-play { display: none; }

  .center-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 20px;
  }

  .btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    color: var(--fg-dark);
  }
  .btn:hover { color: var(--fg); }
  .btn.active { color: var(--accent); }
  .btn.ic-large { width: 40px; height: 40px; color: var(--fg); }

  .playback-bar {
    display: flex;
    align-items: center;
    width: 100%;
  }

  .time {
    font-size: 11px;
    color: var(--comment);
    font-variant-numeric: tabular-nums;
    min-width: 36px;
    text-align: center;
    flex-shrink: 0;
  }

  .progress-wrapper {
    flex: 1;
    height: 12px;
    display: flex;
    align-items: center;
    cursor: pointer;
  }

  .progress-bg {
    width: 100%;
    height: 4px;
    background-color: var(--bg-highlight);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background-color: var(--accent);
    border-radius: 2px;
    pointer-events: none;
  }

  .right-col {
    flex: 0 0 25%;
    display: flex;
    align-items: stretch;
    justify-content: center;
  }

  .right-inner {
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: 10px;
    width: 100%;
  }

  .volume-control {
    display: flex;
    align-items: center;
    color: var(--comment);
    padding-right: 16px;
  }

  .volume-control input { flex: 1; max-width: 100%; accent-color: var(--accent); }

  .vol-row {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .vol-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    font-size: 14px;
    color: var(--comment);
    border-radius: 4px;
  }
  .vol-btn:hover { color: var(--fg); background: var(--bg-highlight); }

  .vol-label {
    font-size: 11px;
    color: var(--comment);
    white-space: nowrap;
    min-width: 28px;
    text-align: center;
  }

  .node-name {
    font-size: 11px;
    color: var(--comment);
    white-space: nowrap;
  }

  @media (max-width: 768px) {
    .center-col, .right-col { display: none; }
    .left-col { flex: 1; }
    .artwork { width: 38px; height: 38px; aspect-ratio: auto; }
    .meta { flex: 1; }
    .mobile-play {
      display: flex;
      align-items: center;
      justify-content: center;
      color: var(--fg);
    }
  }
</style>
