<script lang="ts">
  import { ws } from '../lib/stores';
  import { formatDuration } from '../lib/format';

  let node = $derived(ws.nodes.find(n => n.id === ws.getNodeId()));
  let currentTrack = $derived(node?.queue[node.current_index ?? -1]);
  let isPlaying = $derived(node?.status === 'playing');
  let position = $derived(node?.position_secs ?? 0);
  let duration = $derived(currentTrack?.duration_secs ?? 0);
  let volume = $derived(node?.volume ?? 100);

  function togglePlay() {
    if (!node) return;
    if (isPlaying) {
      ws.sendCommand({ cmd: 'pause', node_id: ws.getNodeId() });
    } else {
      ws.sendCommand({ cmd: 'play', node_id: ws.getNodeId() });
    }
  }

  function playNext() {
    ws.sendCommand({ cmd: 'next', node_id: ws.getNodeId() });
  }

  function playPrev() {
    ws.sendCommand({ cmd: 'previous', node_id: ws.getNodeId() });
  }

  function seek(e: MouseEvent) {
    if (!duration || !node) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const percent = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const newPos = percent * duration;
    ws.sendCommand({ cmd: 'seek', node_id: ws.getNodeId(), position_secs: newPos });
  }

  function setVolume(e: Event) {
    const input = e.target as HTMLInputElement;
    ws.sendCommand({ cmd: 'set_volume', node_id: ws.getNodeId(), volume: parseInt(input.value) });
  }

  function toggleShuffle() {
    if (node) ws.sendCommand({ cmd: 'set_shuffle', node_id: ws.getNodeId(), shuffle: !node.shuffle });
  }

  function toggleRepeat() {
    if (!node) return;
    const map: Record<string, 'off' | 'one' | 'all'> = {
      'off': 'all',
      'all': 'one',
      'one': 'off'
    };
    ws.sendCommand({ cmd: 'set_repeat', node_id: ws.getNodeId(), repeat: map[node.repeat] });
  }
</script>

<div class="transport-bar">
  <div class="track-info">
    {#if currentTrack}
      <div class="title">{currentTrack.title || currentTrack.file_path.split('/').pop()}</div>
      <div class="artist">{currentTrack.artist || 'Unknown Artist'}</div>
    {/if}
  </div>

  <div class="controls-center">
    <div class="buttons">
      <button class="icon sm" class:active={node?.shuffle} onclick={toggleShuffle}>
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 4h9M1 12h9M14 2l-4 4 4 4"/><path d="M10 2l-4 4 4 4"/></svg>
      </button>
      <button class="icon" onclick={playPrev}>
        <svg width="18" height="18" viewBox="0 0 18 18" fill="currentColor"><rect x="1" y="3" width="3" height="12" rx="1"/><path d="M7 3l10 6-10 6V3z"/></svg>
      </button>
      <button class="icon play" onclick={togglePlay}>
        {#if isPlaying}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><rect x="5" y="4" width="4" height="16" rx="1"/><rect x="15" y="4" width="4" height="16" rx="1"/></svg>
        {:else}
          <svg width="24" height="24" viewBox="0 0 24 24" fill="currentColor"><path d="M6 3l16 9-16 9V3z"/></svg>
        {/if}
      </button>
      <button class="icon" onclick={playNext}>
        <svg width="18" height="18" viewBox="0 0 18 18" fill="currentColor"><path d="M4 3l10 6-10 6V3z"/><rect x="14" y="3" width="3" height="12" rx="1"/></svg>
      </button>
      <button class="icon sm" class:active={node?.repeat !== 'off'} onclick={toggleRepeat}>
        {#if node?.repeat === 'one'}
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 8a6 6 0 0112 0"/><path d="M13 5v3h-3"/><text x="7.5" y="11.5" text-anchor="middle" font-size="7" fill="currentColor" stroke="none" font-family="inherit">1</text></svg>
        {:else}
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M1 8a6 6 0 0112 0"/><path d="M13 5v3h-3"/></svg>
        {/if}
      </button>
    </div>

    <div class="progress-container">
      <span class="time">{formatDuration(position)}</span>
      <div class="progress-bar" onclick={seek} role="slider" aria-valuenow={position} tabindex="0">
        <div class="progress-fill" style="width: {(duration ? (position / duration) * 100 : 0)}%"></div>
      </div>
      <span class="time">{formatDuration(duration)}</span>
    </div>
  </div>

  <div class="controls-right">
    <div class="volume">
      <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><path d="M2 5.5v5h3l4 4v-13l-4 4H2z"/><path d="M11 4.5c.8.8 1.3 2 1.3 3.2s-.5 2.4-1.3 3.2" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>
      <input type="range" min="0" max="100" value={volume} onchange={setVolume} />
    </div>
  </div>
</div>

<style>
  .transport-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 100%;
    padding: 0 20px;
  }

  .track-info {
    width: 250px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .title {
    font-weight: 600;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .artist {
    font-size: 12px;
    color: var(--comment);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .controls-center {
    flex: 1;
    max-width: 600px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }

  .buttons {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .icon {
    color: var(--fg-dark);
    transition: color 0.2s;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
  }

  .icon:hover {
    color: var(--fg);
  }

  .icon.active {
    color: var(--accent);
  }

  .icon.sm {
    width: 28px;
    height: 28px;
  }

  .icon.play {
    width: 44px;
    height: 44px;
  }

  .progress-container {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .time {
    font-size: 12px;
    color: var(--comment);
    font-variant-numeric: tabular-nums;
  }

  .progress-bar {
    flex: 1;
    height: 6px;
    background-color: var(--bg-highlight);
    border-radius: 3px;
    cursor: pointer;
    position: relative;
  }

  .progress-bar:hover .progress-fill {
    background-color: var(--accent-hover);
  }

  .progress-fill {
    height: 100%;
    background-color: var(--accent);
    border-radius: 3px;
    pointer-events: none;
  }

  .controls-right {
    width: 250px;
    display: flex;
    justify-content: flex-end;
    align-items: center;
  }

  .volume {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  input[type=range] {
    width: 100px;
    accent-color: var(--accent);
  }
</style>
