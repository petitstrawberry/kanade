<script lang="ts">
  import { localPlayback, localPlaybackState } from '../lib/stores';
  import { formatDuration } from '../lib/format';

  let queue = $derived(localPlaybackState.queue);
  let currentIndex = $derived(localPlaybackState.currentIndex ?? -1);

  function playIndex(index: number) {
    localPlayback.jumpToIndex(index);
  }

  function removeTrack(index: number, e: MouseEvent) {
    e.stopPropagation();
    localPlayback.removeFromQueue(index);
  }

  function moveTrack(from: number, to: number, e: MouseEvent) {
    e.stopPropagation();
    if (to >= 0 && to < queue.length) {
      localPlayback.moveInQueue(from, to);
    }
  }

  function clearQueue() {
    localPlayback.clearQueue();
  }
</script>

<div class="queue-panel">
  <div class="header">
    <h1>Queue</h1>
    <div class="actions">
      <span class="count">{queue.length} tracks</span>
      <button class="clear-btn" onclick={clearQueue}>Clear</button>
    </div>
  </div>

  <div class="track-list">
    {#each queue as track, i}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div 
        class="track-item" 
        class:playing={i === currentIndex}
        onclick={() => playIndex(i)}
      >
        <div class="play-indicator">
          {#if i === currentIndex}
            <span class="icon playing-icon">▶</span>
          {:else}
            <span class="index">{i + 1}</span>
            <button class="icon play-btn" onclick={(e) => { e.stopPropagation(); playIndex(i); }}>▶</button>
          {/if}
        </div>

        <div class="track-info">
          <div class="title">{track.title || track.file_path.split('/').pop()}</div>
          <div class="artist">{track.artist || 'Unknown Artist'}</div>
        </div>

        <div class="duration">
          {formatDuration(track.duration_secs)}
        </div>

        <div class="controls">
          <button class="icon-btn" onclick={(e) => moveTrack(i, i - 1, e)} disabled={i === 0}>↑</button>
          <button class="icon-btn" onclick={(e) => moveTrack(i, i + 1, e)} disabled={i === queue.length - 1}>↓</button>
          <button class="icon-btn remove" onclick={(e) => removeTrack(i, e)} aria-label="Remove">×</button>
        </div>
      </div>
    {/each}
    {#if queue.length === 0}
      <div class="empty">The queue is empty</div>
    {/if}
  </div>
</div>

<style>
  .queue-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 24px;
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    margin-bottom: 24px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--bg-highlight);
  }

  h1 {
    font-size: 32px;
    color: var(--fg);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .count {
    color: var(--comment);
    font-size: 14px;
  }

  .clear-btn {
    padding: 6px 12px;
    background-color: var(--bg-highlight);
    border-radius: 6px;
    color: var(--fg-dark);
    font-size: 14px;
  }

  .clear-btn:hover {
    background-color: var(--red);
    color: var(--bg-dark);
  }

  .track-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .track-item {
    display: flex;
    align-items: center;
    padding: 12px 16px;
    border-radius: 8px;
    gap: 16px;
    transition: background-color 0.2s;
  }

  .track-item:hover {
    background-color: var(--bg-highlight);
  }

  .track-item.playing {
    background-color: var(--bg-highlight);
  }

  .track-item.playing .title {
    color: var(--accent);
  }

  .play-indicator {
    width: 30px;
    display: flex;
    justify-content: center;
    color: var(--comment);
    font-size: 14px;
  }

  .playing-icon {
    color: var(--accent);
  }

  .play-btn {
    display: none;
    color: var(--fg);
  }

  .track-item:hover .index {
    display: none;
  }

  .track-item:hover .play-btn {
    display: block;
  }

  .track-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .title {
    color: var(--fg);
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .artist {
    color: var(--comment);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .duration {
    color: var(--comment);
    font-variant-numeric: tabular-nums;
    font-size: 14px;
  }

  .controls {
    display: flex;
    gap: 8px;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .track-item:hover .controls {
    opacity: 1;
  }

  .icon-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    background-color: var(--bg-dark);
    color: var(--fg-dark);
  }

  .icon-btn:hover:not(:disabled) {
    background-color: var(--accent);
    color: var(--bg);
  }

  .icon-btn.remove:hover {
    background-color: var(--red);
  }

  .icon-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .empty {
    padding: 40px;
    text-align: center;
    color: var(--comment);
  }

  @media (max-width: 768px) {
    .queue-panel {
      padding: 12px;
    }

    .track-item {
      padding: 8px;
      gap: 12px;
      min-height: 44px;
    }

    .controls {
      opacity: 1;
    }

    .icon-btn {
      min-width: 44px;
      min-height: 44px;
    }
  }
</style>
