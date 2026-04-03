<script lang="ts">
  import { ws, nodeId, mediaBase } from '../lib/stores';
  import { formatSampleRate } from '../lib/format';

  let zone = $derived(ws.nodes.find(z => z.id === nodeId));
  let currentTrack = $derived(zone?.queue[zone.current_index ?? -1]);
  let artworkUrl = $derived(currentTrack?.album_id ? `${mediaBase}/media/art/${currentTrack.album_id}` : null);
</script>

<div class="now-playing">
  {#if currentTrack}
    <div class="info-wrapper">
      <div class="cover-art">
        {#if artworkUrl}
          <img src={artworkUrl} alt={currentTrack.album_title || 'Album'} />
          <div class="cover-fallback" style="display:none">
            <svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
          </div>
        {:else}
          <div class="cover-fallback">
            <svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle></svg>
          </div>
        {/if}
      </div>

      <div class="details">
        <h1 class="title">{currentTrack.title || currentTrack.file_path.split('/').pop()}</h1>
        <h2 class="artist">{currentTrack.artist || 'Unknown Artist'}</h2>
        
        <div class="meta">
          {#if currentTrack.album_title}
            <div class="meta-item">
              <span class="label">Album</span>
              <span class="value">{currentTrack.album_title}</span>
            </div>
          {/if}
          
          {#if currentTrack.composer}
            <div class="meta-item">
              <span class="label">Composer</span>
              <span class="value">{currentTrack.composer}</span>
            </div>
          {/if}

          {#if currentTrack.genre}
            <div class="meta-item">
              <span class="label">Genre</span>
              <span class="value">{currentTrack.genre}</span>
            </div>
          {/if}

          <div class="format-info">
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
  {:else}
    <div class="empty-state">
      <div class="empty-icon">
        <svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></svg>
      </div>
      <p>No track is currently playing.</p>
    </div>
  {/if}
</div>

<style>
  .now-playing {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px;
    background: radial-gradient(circle at 50% 0%, var(--bg-highlight), var(--bg));
  }

  .info-wrapper {
    display: flex;
    align-items: center;
    gap: 60px;
    max-width: 1000px;
    width: 100%;
  }

  .cover-art {
    width: 300px;
    height: 300px;
    background: linear-gradient(135deg, var(--bg-highlight), var(--bg-dark));
    border-radius: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.4);
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

  .details {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .title {
    font-size: 48px;
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
    font-size: 24px;
    font-weight: 500;
    color: var(--accent);
    margin-bottom: 32px;
  }

  .meta {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .meta-item {
    display: flex;
    gap: 16px;
    font-size: 16px;
  }

  .label {
    color: var(--comment);
    width: 100px;
    text-transform: uppercase;
    font-size: 12px;
    letter-spacing: 0.1em;
    padding-top: 2px;
  }

  .value {
    color: var(--fg-dark);
    flex: 1;
  }

  .format-info {
    display: flex;
    gap: 8px;
    margin-top: 24px;
  }

  .format-badge {
    background-color: var(--bg-dark);
    border: 1px solid var(--fg-gutter);
    color: var(--cyan);
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 12px;
    font-weight: bold;
    letter-spacing: 0.05em;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    color: var(--comment);
  }

  .empty-icon {
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
