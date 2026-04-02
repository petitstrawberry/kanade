<script lang="ts">
  import { ws, zoneId } from '../lib/stores';
  import type { Album, Track } from '../lib/types';
  import { formatDuration } from '../lib/format';

  type Mode = 'albums' | 'artists' | 'genres';
  let mode = $state<Mode>('albums');

  let albums = $state<Album[]>([]);
  let artists = $state<string[]>([]);
  let genres = $state<string[]>([]);

  let selectedArtist = $state<string | null>(null);
  let selectedGenre = $state<string | null>(null);
  let selectedAlbum = $state<Album | null>(null);
  
  let currentTracks = $state<Track[]>([]);

  function cycleMode(reverse: boolean) {
    const modes: Mode[] = ['albums', 'artists', 'genres'];
    const idx = modes.indexOf(mode);
    let nextIdx = reverse ? idx - 1 : idx + 1;
    if (nextIdx < 0) nextIdx = modes.length - 1;
    if (nextIdx >= modes.length) nextIdx = 0;
    mode = modes[nextIdx];
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement) return;
    if (e.key === 'm') {
      cycleMode(false);
    } else if (e.key === 'M') {
      cycleMode(true);
    }
  }

  function loadRoot() {
    selectedAlbum = null;
    selectedArtist = null;
    selectedGenre = null;
    currentTracks = [];

    if (mode === 'albums') {
      ws.sendRequest({ req: 'get_albums' }).then(res => {
        if ('albums' in res) albums = res.albums;
      }).catch(() => {});
    } else if (mode === 'artists') {
      ws.sendRequest({ req: 'get_artists' }).then(res => {
        if ('artists' in res) artists = res.artists;
      }).catch(() => {});
    } else if (mode === 'genres') {
      ws.sendRequest({ req: 'get_genres' }).then(res => {
        if ('genres' in res) genres = res.genres;
      }).catch(() => {});
    }
  }

  $effect(() => {
    ws.connected;
    mode;
    loadRoot();
  });

  function selectArtist(artist: string) {
    selectedArtist = artist;
    selectedAlbum = null;
    currentTracks = [];
    ws.sendRequest({ req: 'get_artist_albums', artist }).then(res => {
      if ('albums' in res) albums = res.albums;
    }).catch(() => {});
  }

  function selectGenre(genre: string) {
    selectedGenre = genre;
    selectedAlbum = null;
    currentTracks = [];
    ws.sendRequest({ req: 'get_genre_albums', genre }).then(res => {
      if ('albums' in res) albums = res.albums;
    }).catch(() => {});
  }

  function selectAlbum(album: Album) {
    selectedAlbum = album;
    ws.sendRequest({ req: 'get_album_tracks', album_id: album.id }).then(res => {
      if ('tracks' in res) currentTracks = res.tracks;
    }).catch(() => {});
  }

  function goBack() {
    if (selectedAlbum && (selectedArtist || selectedGenre)) {
      selectedAlbum = null;
      currentTracks = [];
      if (selectedArtist) selectArtist(selectedArtist);
      else if (selectedGenre) selectGenre(selectedGenre);
    } else {
      loadRoot();
    }
  }

  function addToQueue(track: Track) {
    ws.sendCommand({ cmd: 'add_to_queue', zone_id: zoneId, track });
  }

  function playNow(track: Track) {
    ws.sendCommand({ cmd: 'add_to_queue', zone_id: zoneId, track });
    const queueLen = ws.zones.find(z => z.id === zoneId)?.queue.length ?? 0;
    ws.sendCommand({ cmd: 'play_index', zone_id: zoneId, index: queueLen });
  }

  function addAlbumToQueue(albumId: string) {
    ws.sendRequest({ req: 'get_album_tracks', album_id: albumId }).then(res => {
      if ('tracks' in res) {
        ws.sendCommand({ cmd: 'add_tracks_to_queue', zone_id: zoneId, tracks: res.tracks });
      }
    }).catch(() => {});
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="library-panel">
  <div class="header">
    <div class="mode-switcher">
      <button class:active={mode === 'albums'} onclick={() => mode = 'albums'}>Albums</button>
      <button class:active={mode === 'artists'} onclick={() => mode = 'artists'}>Artists</button>
      <button class:active={mode === 'genres'} onclick={() => mode = 'genres'}>Genres</button>
    </div>
  </div>

  <div class="content">
    {#if selectedAlbum || selectedArtist || selectedGenre}
      <div class="breadcrumb">
        <button onclick={goBack} class="back-btn">← Back</button>
        <span class="path">
          {mode.charAt(0).toUpperCase() + mode.slice(1)}
          {#if selectedArtist} / {selectedArtist}{/if}
          {#if selectedGenre} / {selectedGenre}{/if}
          {#if selectedAlbum} / {selectedAlbum.title || 'Unknown Album'}{/if}
        </span>
      </div>
    {/if}

    <div class="view-area">
      <!-- List View (Albums, Artists, or Genres grid) -->
      {#if !selectedAlbum}
        <div class="list-pane">
          {#if mode === 'artists' && !selectedArtist}
            <div class="grid-list">
              {#each artists as artist}
                <button class="list-item" onclick={() => selectArtist(artist)}>
                  {artist || 'Unknown Artist'}
                </button>
              {/each}
            </div>
          {:else if mode === 'genres' && !selectedGenre}
            <div class="grid-list">
              {#each genres as genre}
                <button class="list-item" onclick={() => selectGenre(genre)}>
                  {genre || 'Unknown Genre'}
                </button>
              {/each}
            </div>
          {:else}
            <!-- Albums grid -->
            <div class="album-grid">
              {#each albums as album}
                <div class="album-card">
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div class="album-cover" onclick={() => selectAlbum(album)}>
                    <div class="placeholder-icon">
                      <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
                    </div>
                    <div class="play-overlay">
                      <button class="add-btn" onclick={(e) => { e.stopPropagation(); addAlbumToQueue(album.id); }}>+</button>
                    </div>
                  </div>
                  <div class="album-info" onclick={() => selectAlbum(album)}>
                    <div class="album-title">{album.title || 'Unknown Album'}</div>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {:else}
        <!-- Tracks View -->
        <div class="tracks-pane">
          <div class="tracks-header">
            <div class="album-info-header">
              <h2>{selectedAlbum.title || 'Unknown Album'}</h2>
              {#if currentTracks.length > 0 && currentTracks[0].album_artist}
                <div class="artist-subtitle">{currentTracks[0].album_artist}</div>
              {/if}
              <div class="album-meta">
                {currentTracks.length} tracks • {formatDuration(currentTracks.reduce((acc, t) => acc + (t.duration_secs || 0), 0))}
              </div>
            </div>
          </div>
          <div class="track-list">
            {#each currentTracks as track}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="track-item" ondblclick={() => playNow(track)} onclick={() => addToQueue(track)}>
                <div class="track-number">{track.track_number || '-'}</div>
                <div class="track-info">
                  <div class="title">{track.title || track.file_path.split('/').pop()}</div>
                  {#if track.artist && track.artist !== track.album_artist}
                    <div class="track-artist">{track.artist}</div>
                  {/if}
                </div>
                <div class="duration">{formatDuration(track.duration_secs)}</div>
                <button class="add-btn" onclick={(e) => { e.stopPropagation(); addToQueue(track); }}>+</button>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .library-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 24px;
  }

  .header {
    margin-bottom: 24px;
  }

  .mode-switcher {
    display: inline-flex;
    background-color: var(--bg-dark);
    padding: 4px;
    border-radius: 8px;
    border: 1px solid var(--bg-highlight);
  }

  .mode-switcher button {
    padding: 8px 16px;
    border-radius: 6px;
    color: var(--comment);
    font-weight: 500;
  }

  .mode-switcher button:hover {
    color: var(--fg);
  }

  .mode-switcher button.active {
    background-color: var(--bg-highlight);
    color: var(--accent);
  }

  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 20px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--bg-highlight);
  }

  .back-btn {
    padding: 6px 12px;
    background-color: var(--bg-dark);
    border-radius: 6px;
    color: var(--fg);
  }

  .back-btn:hover {
    background-color: var(--bg-highlight);
  }

  .path {
    color: var(--comment);
    font-size: 14px;
  }

  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .view-area {
    flex: 1;
    display: flex;
    gap: 24px;
    min-height: 0;
  }

  .list-pane, .tracks-pane {
    flex: 1;
    overflow-y: auto;
  }

  .grid-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 12px;
  }

  .list-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px;
    background-color: var(--bg-dark);
    border-radius: 8px;
    color: var(--fg);
    text-align: left;
    transition: background-color 0.2s;
  }

  .list-item:hover {
    background-color: var(--bg-highlight);
  }

  .album-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 24px;
  }

  .album-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .album-cover {
    aspect-ratio: 1;
    background: linear-gradient(135deg, var(--bg-highlight), var(--bg-dark));
    border-radius: 8px;
    position: relative;
    cursor: pointer;
    overflow: hidden;
  }

  .placeholder-icon {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--fg);
    opacity: 0.15;
  }

  .play-overlay {
    position: absolute;
    inset: 0;
    background-color: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .album-cover:hover .play-overlay {
    opacity: 1;
  }

  .play-overlay .add-btn {
    width: 48px;
    height: 48px;
    font-size: 24px;
    opacity: 1;
  }

  .album-info {
    cursor: pointer;
  }

  .album-title {
    font-weight: 500;
    color: var(--fg);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .tracks-header {
    position: sticky;
    top: 0;
    background-color: var(--bg);
    z-index: 10;
    padding-bottom: 24px;
    margin-bottom: 16px;
    border-bottom: 1px solid var(--bg-highlight);
  }

  .album-info-header h2 {
    margin: 0 0 8px 0;
    font-size: 28px;
    color: var(--fg);
    line-height: 1.2;
  }

  .artist-subtitle {
    font-size: 16px;
    color: var(--accent);
    margin-bottom: 8px;
    font-weight: 500;
  }

  .album-meta {
    font-size: 14px;
    color: var(--comment);
  }

  .track-list {
    display: flex;
    flex-direction: column;
  }

  .track-item {
    display: flex;
    align-items: center;
    padding: 10px 12px;
    border-radius: 6px;
    gap: 16px;
    cursor: pointer;
  }

  .track-item:hover {
    background-color: var(--bg-highlight);
  }

  .track-number {
    width: 24px;
    color: var(--comment);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .track-info {
    flex: 1;
    min-width: 0;
  }

  .track-info .title {
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .track-artist {
    font-size: 13px;
    color: var(--comment);
    margin-top: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .duration {
    color: var(--comment);
    font-variant-numeric: tabular-nums;
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
