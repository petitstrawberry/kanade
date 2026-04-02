<script lang="ts">
  import { ws, zoneId } from '../lib/stores';
  import type { Track } from '../lib/types';
  import { formatDuration } from '../lib/format';

  let query = $state('');
  let results = $state<Track[]>([]);
  let searching = $state(false);
  let searchTimeout: number | null = null;

  function performSearch() {
    if (!query.trim()) {
      results = [];
      return;
    }
    
    searching = true;
    ws.sendRequest({ req: 'search', query }).then(res => {
      if ('search_results' in res) {
        results = res.search_results;
      }
      searching = false;
    }).catch(err => {
      console.error(err);
      searching = false;
    });
  }

  function handleInput(e: Event) {
    query = (e.target as HTMLInputElement).value;
    if (searchTimeout) clearTimeout(searchTimeout);
    searchTimeout = window.setTimeout(performSearch, 300);
  }

  function addToQueue(track: Track) {
    ws.sendCommand({ cmd: 'add_to_queue', zone_id: zoneId, track });
  }

  function playNow(track: Track) {
    ws.sendCommand({ cmd: 'add_to_queue', zone_id: zoneId, track });
    const queueLen = ws.zones.find(z => z.id === zoneId)?.queue.length ?? 0;
    ws.sendCommand({ cmd: 'play_index', zone_id: zoneId, index: queueLen }); // approximate index
  }
</script>

<div class="search-panel">
  <div class="header">
    <input 
      type="text" 
      class="search-input" 
      placeholder="Search for tracks, artists, albums..." 
      value={query}
      oninput={handleInput}
      autofocus
    />
  </div>

  <div class="results">
    {#if searching}
      <div class="message">Searching...</div>
    {:else if query && results.length === 0}
      <div class="message">No results found for "{query}"</div>
    {:else if !query}
      <div class="message">Type to start searching</div>
    {:else}
      <div class="track-list">
        {#each results as track, i}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="track-item" ondblclick={() => playNow(track)} onclick={() => addToQueue(track)}>
            <div class="track-info">
              <div class="title">{track.title || track.file_path.split('/').pop()}</div>
              <div class="meta">
                <span>{track.artist || 'Unknown Artist'}</span>
                {#if track.album_title}
                  <span class="dot">•</span>
                  <span>{track.album_title}</span>
                {/if}
              </div>
            </div>
            
            <div class="duration">
              {formatDuration(track.duration_secs)}
            </div>

            <button class="add-btn" onclick={(e) => { e.stopPropagation(); addToQueue(track); }}>
              +
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .search-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 24px;
  }

  .header {
    margin-bottom: 24px;
  }

  .search-input {
    width: 100%;
    background-color: var(--bg-dark);
    border: 2px solid var(--bg-highlight);
    border-radius: 8px;
    padding: 16px 20px;
    font-size: 18px;
    color: var(--fg);
    outline: none;
    transition: border-color 0.2s;
  }

  .search-input:focus {
    border-color: var(--accent);
  }

  .search-input::placeholder {
    color: var(--comment);
  }

  .results {
    flex: 1;
    overflow-y: auto;
  }

  .message {
    padding: 40px;
    text-align: center;
    color: var(--comment);
    font-size: 18px;
  }

  .track-list {
    display: flex;
    flex-direction: column;
  }

  .track-item {
    display: flex;
    align-items: center;
    padding: 12px 16px;
    border-radius: 8px;
    gap: 16px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .track-item:hover {
    background-color: var(--bg-highlight);
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

  .meta {
    color: var(--comment);
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dot {
    font-size: 10px;
  }

  .duration {
    color: var(--comment);
    font-variant-numeric: tabular-nums;
    font-size: 14px;
  }

  .add-btn {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background-color: var(--bg-dark);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 20px;
    opacity: 0;
    transition: all 0.2s;
  }

  .track-item:hover .add-btn {
    opacity: 1;
  }

  .add-btn:hover {
    background-color: var(--accent);
    color: var(--bg);
    transform: scale(1.1);
  }
</style>
