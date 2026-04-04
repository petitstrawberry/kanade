<script lang="ts">
  import { ws, showToast } from '../lib/stores';
  import type { Album, Track } from '../lib/types';
  import { formatDuration } from '../lib/format';
  import { mediaBase } from '../lib/stores';
  import { tick } from 'svelte';

  type Mode = 'albums' | 'artists' | 'genres';
  let mode = $state<Mode>('albums');
  let sizeIndex = $state(parseInt(localStorage.getItem('kanade-artwork-size-idx') || '1'));
  const sizePresets = [120, 180, 260, 360];
  let artworkSize = $derived(sizePresets[sizeIndex]);
  $effect(() => { localStorage.setItem('kanade-artwork-size-idx', String(sizeIndex)); });

  const scrollStore = new Map<string, number>();
  let viewEl: HTMLElement;

  const PLACEHOLDER_SVG = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" viewBox="0 0 24 24" fill="none" stroke="%23888" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect width="24" height="24" fill="%231a1b26"/><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>')}`;

  let albums = $state<Album[]>([]);
  let artists = $state<string[]>([]);
  let genres = $state<string[]>([]);

  let selectedArtist = $state<string | null>(null);
  let selectedGenre = $state<string | null>(null);
  let selectedAlbum = $state<Album | null>(null);

  let currentTracks = $state<Track[]>([]);

  const hasMultipleDiscs = $derived(
    new Set(currentTracks.map(t => t.disc_number ?? 0)).size > 1
  );

  let viewKey = $derived(
    selectedAlbum ? `album-${selectedAlbum.id}`
    : selectedArtist ? `artist-${selectedArtist}`
    : selectedGenre ? `genre-${selectedGenre}`
    : `root-${mode}`
  );

  $effect(() => {
    viewKey;
    tick().then(() => {
      if (viewEl) viewEl.scrollTop = scrollStore.get(viewKey) ?? 0;
    });
  });

  function saveScroll() {
    if (viewEl) scrollStore.set(viewKey, viewEl.scrollTop);
  }

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
    albums = [];
    ws.sendRequest({ req: 'get_artist_albums', artist }).then(res => {
      if ('albums' in res) albums = res.albums;
    }).catch(() => {});
  }

  function selectGenre(genre: string) {
    selectedGenre = genre;
    selectedAlbum = null;
    currentTracks = [];
    albums = [];
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
    ws.sendCommand({ cmd: 'add_to_queue', track });
    showToast(`Added: ${track.title || 'Track'}`);
  }

  function playNow(track: Track, tracks: Track[], index: number) {
    ws.sendCommand({ cmd: 'replace_and_play', tracks, index });
  }

  function addAlbumTracksToQueue(albumId: string) {
    ws.sendRequest({ req: 'get_album_tracks', album_id: albumId }).then(res => {
      if ('tracks' in res) {
        ws.sendCommand({ cmd: 'add_tracks_to_queue', tracks: res.tracks });
        showToast(`Added ${res.tracks.length} tracks`);
      }
    }).catch(() => {});
  }

  function playAlbumFromGrid(album: Album) {
    ws.sendRequest({ req: 'get_album_tracks', album_id: album.id }).then(res => {
      if ('tracks' in res && res.tracks.length > 0) {
        ws.sendCommand({ cmd: 'replace_and_play', tracks: res.tracks, index: 0 });
      }
    }).catch(() => {});
  }

  function addAlbumToQueue() {
    ws.sendCommand({ cmd: 'add_tracks_to_queue', tracks: currentTracks });
    showToast(`Added ${currentTracks.length} tracks`);
  }

  function playAlbumNow() {
    ws.sendCommand({ cmd: 'replace_and_play', tracks: currentTracks, index: 0 });
  }
</script>

<svelte:window onkeydown={handleKeydown} />

  <div class="library-panel" style="--artwork-size: {artworkSize}px">
    <div class="content">
      <div class="header">
        {#if !selectedAlbum && !selectedArtist && !selectedGenre}
          <div class="mode-switcher">
            <button class:active={mode === 'albums'} onclick={() => mode = 'albums'}>Albums</button>
            <button class:active={mode === 'artists'} onclick={() => mode = 'artists'}>Artists</button>
            <button class:active={mode === 'genres'} onclick={() => mode = 'genres'}>Genres</button>
          </div>
          <button class="size-btn" onclick={() => sizeIndex = (sizeIndex + 1) % 4}>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="2" width="5" height="5"/><rect x="9" y="2" width="5" height="5"/><rect x="2" y="9" width="5" height="5"/><rect x="9" y="9" width="5" height="5"/></svg>
          </button>
        {:else}
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
      </div>

      <div class="view-area" bind:this={viewEl} onscroll={saveScroll}>
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
                      <img
                        src="{mediaBase}/media/art/{album.id}"
                        alt={album.title || 'Album'}
                        onerror={(e: Event) => {
                          const img = e.target as HTMLImageElement;
                          if (!img.dataset.fallback) {
                            img.dataset.fallback = '1';
                            img.src = PLACEHOLDER_SVG;
                          }
                        }}
                      />
                      <div class="play-overlay">
                        <button class="add-btn" onclick={(e) => { e.stopPropagation(); addAlbumTracksToQueue(album.id); }}>+</button>
                        <button class="play-btn" onclick={(e) => { e.stopPropagation(); playAlbumFromGrid(album); }}>▶</button>
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
            <img
              class="album-art"
              src="{mediaBase}/media/art/{selectedAlbum.id}"
              alt=""
              onerror={(e: Event) => {
                const img = e.target as HTMLImageElement;
                if (!img.dataset.fallback) {
                  img.dataset.fallback = '1';
                  img.src = PLACEHOLDER_SVG;
                }
              }}
            />
            <div class="album-info-header">
              <h2>{selectedAlbum.title || 'Unknown Album'}</h2>
              {#if currentTracks.length > 0 && currentTracks[0].album_artist}
                <div class="artist-subtitle">{currentTracks[0].album_artist}</div>
              {/if}
              <div class="album-meta">
                {currentTracks.length} tracks • {formatDuration(currentTracks.reduce((acc, t) => acc + (t.duration_secs || 0), 0))}
              </div>
              <div class="album-actions">
                <button class="action-btn" onclick={playAlbumNow}>▶ Play</button>
                <button class="action-btn" onclick={addAlbumToQueue}>+ Add All</button>
              </div>
            </div>
          </div>
          <div class="track-list">
            {#each currentTracks as track, i}
              {#if hasMultipleDiscs && (i === 0 || track.disc_number !== currentTracks[i - 1].disc_number)}
                <div class="disc-separator">Disc {track.disc_number ?? 1}</div>
              {/if}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="track-item" onclick={() => playNow(track, currentTracks, i)}>
                <div class="track-number">{track.track_number ?? '-'}</div>
                <div class="track-info">
                  <div class="title">{track.title || track.file_path.split('/').pop()}</div>
                  {#if track.artist && track.artist !== track.album_artist}
                    <div class="track-artist">{track.artist}</div>
                  {/if}
                </div>
                <div class="duration">{formatDuration(track.duration_secs)}</div>
                <div class="track-actions">
                  <button class="add-btn" onclick={(e) => { e.stopPropagation(); addToQueue(track); }}>+</button>
                </div>
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
    height: 40px;
    margin-top: 8px;
    margin-bottom: 20px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--bg-highlight);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .size-btn {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    color: var(--comment);
    border-radius: 4px;
    cursor: pointer;
  }

  .size-btn:hover {
    color: var(--fg);
  }

  .mode-switcher {
    display: flex;
    gap: 16px;
  }

  .mode-switcher button {
    padding: 4px 0;
    color: var(--comment);
    font-weight: 500;
    font-size: 14px;
    background: transparent;
  }

  .mode-switcher button:hover {
    color: var(--fg);
  }

  .mode-switcher button.active {
    color: var(--accent);
  }

  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .back-btn {
    padding: 4px 0;
    background: transparent;
    color: var(--fg);
    font-weight: 500;
    font-size: 14px;
  }

  .back-btn:hover {
    color: var(--comment);
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
    overflow-y: auto;
    min-height: 0;
  }

  .list-pane {
    flex: 1;
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
    grid-template-columns: repeat(auto-fill, minmax(var(--artwork-size), 1fr));
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

  .album-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
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

  .play-overlay .add-btn,
  .play-overlay .play-btn {
    width: 48px;
    height: 48px;
    font-size: 24px;
    opacity: 1;
  }

  .play-overlay .play-btn {
    border-radius: 50%;
    background-color: var(--accent);
    color: var(--bg);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    border: none;
    margin-left: 8px;
    transition: transform 0.2s;
  }

  .play-overlay .play-btn:hover {
    transform: scale(1.1);
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
    padding-bottom: 24px;
    margin-bottom: 16px;
    border-bottom: 1px solid var(--bg-highlight);
    display: flex;
    gap: 20px;
    align-items: flex-start;
  }

  .album-art {
    width: var(--artwork-size);
    height: var(--artwork-size);
    border-radius: 8px;
    object-fit: cover;
    flex-shrink: 0;
  }

  .album-info-header {
    flex: 1;
    min-width: 0;
  }

  .album-actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }

  .action-btn {
    padding: 8px 16px;
    border-radius: 6px;
    background-color: var(--bg-dark);
    color: var(--accent);
    font-weight: 500;
    font-size: 13px;
    opacity: 1;
    cursor: pointer;
    border: none;
    transition: all 0.2s;
  }

  .action-btn:hover {
    background-color: var(--accent);
    color: var(--bg);
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

  .track-actions {
    display: flex;
    gap: 4px;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .track-item:hover .track-actions {
    opacity: 1;
  }

  .track-actions .play-btn {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background-color: var(--bg-dark);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    cursor: pointer;
    border: none;
  }

  .track-actions .play-btn:hover {
    background-color: var(--accent);
    color: var(--bg);
  }

  .disc-separator {
    padding: 8px 12px 4px;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--accent);
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

  @media (max-width: 768px) {
    .library-panel {
      padding: 12px 12px 0;
    }

    .header {
      margin-top: 12px;
    }

    .album-grid {
      gap: 16px;
    }

    .grid-list {
      grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    }

    .mode-switcher button {
      min-height: 44px;
    }

    .back-btn {
      min-height: 44px;
    }

    .list-item {
      min-height: 44px;
      padding: 12px;
    }

    .tracks-header {
      flex-direction: column;
      align-items: center;
      text-align: center;
    }

    .album-actions {
      justify-content: center;
    }

    .action-btn {
      min-height: 44px;
      min-width: 44px;
      display: flex;
      align-items: center;
      justify-content: center;
    }

    .track-item {
      padding: 8px;
      gap: 12px;
      min-height: 44px;
    }

    .track-actions {
      opacity: 1;
    }

    .add-btn {
      opacity: 1;
      width: 44px;
      height: 44px;
    }

    .track-item:hover .add-btn {
      transform: none;
    }
  }

  @media (min-width: 769px) and (max-width: 1024px) {
    .album-grid {
      grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    }
  }
</style>
