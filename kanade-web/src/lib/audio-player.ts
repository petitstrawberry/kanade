export interface PlayerState {
  status: 'stopped' | 'playing' | 'paused' | 'loading';
  positionSecs: number;
  volume: number;
  currentSongIndex: number;
  projectionGeneration: number;
}

export interface TrackMetadata {
  title: string;
  artist: string;
  album: string;
  artworkUrl: string | null;
}

export type StateChangeCallback = (state: PlayerState) => void;

export class AudioPlayer {
  private readonly audio: HTMLAudioElement;
  private readonly onStateChange: StateChangeCallback;
  private playlist: string[] = [];
  private state: PlayerState = {
    status: 'stopped',
    positionSecs: 0,
    volume: 100,
    currentSongIndex: 0,
    projectionGeneration: 0,
  };
  private lastEmittedState: PlayerState = { ...this.state };
  private lastPositionEmitAt = 0;

  private readonly handleTimeUpdate = () => {
    this.state.positionSecs = this.audio.currentTime;
    const now = Date.now();
    if (now - this.lastPositionEmitAt >= 500) {
      this.lastPositionEmitAt = now;
      this.emitState();
    }
  };

  private readonly handleEnded = () => {
    if (this.state.currentSongIndex + 1 < this.playlist.length) {
      this.state.currentSongIndex += 1;
      this.state.positionSecs = 0;
      this.state.status = 'loading';
      this.audio.src = this.playlist[this.state.currentSongIndex];
      this.emitState(true);
      void this.audio.play().catch((err) => {
        console.error('Audio play failed:', err);
      this.state.status = 'stopped';
        this.emitState(true);
      });
      return;
    }

    this.state.status = 'stopped';
    this.state.positionSecs = this.audio.currentTime;
    this.emitState(true);
  };

  private readonly handlePlay = () => {
    this.state.status = 'playing';
    this.emitState(true);
  };

  private readonly handlePause = () => {
    if (this.state.status !== 'stopped') {
      this.state.status = 'paused';
    }
    this.emitState(true);
  };

  private readonly handleWaiting = () => {
    this.state.status = 'loading';
    this.emitState(true);
  };

  private readonly handleCanPlay = () => {
    if (this.state.status === 'loading') {
      this.state.status = this.audio.paused ? 'paused' : 'playing';
      this.emitState(true);
    }
  };

  private readonly handleError = () => {
    console.error('Audio element error:', this.audio.error);
    this.state.status = 'stopped';
    this.emitState(true);
  };

  constructor(onStateChange: StateChangeCallback) {
    this.onStateChange = onStateChange;
    this.audio = new Audio();
    this.audio.preload = 'auto';
    this.audio.volume = 1;
    this.audio.muted = false;

    this.audio.addEventListener('timeupdate', this.handleTimeUpdate);
    this.audio.addEventListener('ended', this.handleEnded);
    this.audio.addEventListener('play', this.handlePlay);
    this.audio.addEventListener('pause', this.handlePause);
    this.audio.addEventListener('waiting', this.handleWaiting);
    this.audio.addEventListener('canplay', this.handleCanPlay);
    this.audio.addEventListener('error', this.handleError);

    if ('mediaSession' in navigator) {
      navigator.mediaSession.setActionHandler('play', () => this.play());
      navigator.mediaSession.setActionHandler('pause', () => this.pause());
      navigator.mediaSession.setActionHandler('seekto', (details) => {
        if (details.seekTime != null) this.seek(details.seekTime);
      });
    }
  }

  async primePlayback(): Promise<void> {
    if (!this.audio.src && this.playlist.length > 0) {
      this.audio.src = this.playlist[this.state.currentSongIndex] ?? this.playlist[0];
    }
    try {
      await this.audio.play();
      this.audio.pause();
      this.audio.currentTime = 0;
    } catch {
    }
  }

  setQueue(filePaths: string[], projectionGeneration: number): void {
    console.log('AudioPlayer.setQueue', { filePaths, projectionGeneration });
    this.playlist = [...filePaths];
    this.state.currentSongIndex = 0;
    this.state.projectionGeneration = projectionGeneration;
    this.state.positionSecs = 0;

    if (this.playlist.length === 0) {
      this.audio.pause();
      this.audio.removeAttribute('src');
      this.audio.load();
      this.state.status = 'stopped';
      this.emitState(true);
      return;
    }

    this.audio.pause();
    this.audio.src = this.playlist[0];
    console.log('AudioPlayer.src', this.audio.src);
    this.state.status = 'loading';
    this.emitState(true);
  }

  addTracks(filePaths: string[]): void {
    if (filePaths.length === 0) return;
    this.playlist.push(...filePaths);
  }

  removeTrack(index: number): void {
    if (index < 0 || index >= this.playlist.length) return;

    const removingCurrent = index === this.state.currentSongIndex;
    this.playlist.splice(index, 1);

    if (this.playlist.length === 0) {
      this.stop();
      this.audio.removeAttribute('src');
      this.audio.load();
      this.state.currentSongIndex = 0;
      this.emitState(true);
      return;
    }

    if (index < this.state.currentSongIndex) {
      this.state.currentSongIndex -= 1;
      this.emitState(true);
      return;
    }

    if (removingCurrent) {
      if (this.state.currentSongIndex >= this.playlist.length) {
        this.state.currentSongIndex = this.playlist.length - 1;
      }

      this.state.positionSecs = 0;
      this.audio.src = this.playlist[this.state.currentSongIndex];
      this.state.status = 'loading';
      this.emitState(true);
    }
  }

  moveTrack(from: number, to: number): void {
    if (from < 0 || to < 0 || from >= this.playlist.length || to >= this.playlist.length || from === to) {
      return;
    }

    const [track] = this.playlist.splice(from, 1);
    this.playlist.splice(to, 0, track);

    if (this.state.currentSongIndex === from) {
      this.state.currentSongIndex = to;
      this.emitState(true);
      return;
    }

    if (from < this.state.currentSongIndex && to >= this.state.currentSongIndex) {
      this.state.currentSongIndex -= 1;
      this.emitState(true);
      return;
    }

    if (from > this.state.currentSongIndex && to <= this.state.currentSongIndex) {
      this.state.currentSongIndex += 1;
      this.emitState(true);
    }
  }

  play(): void {
    if (this.playlist.length === 0) return;
    if (!this.audio.src) {
      this.audio.src = this.playlist[this.state.currentSongIndex] ?? this.playlist[0];
    }
    console.log('AudioPlayer.play', { src: this.audio.src, index: this.state.currentSongIndex, playlistLength: this.playlist.length });
    this.state.status = 'loading';
    this.emitState(true);
    void this.audio.play().catch((err) => {
      console.error('Audio play failed:', err);
      this.state.status = 'stopped';
      this.emitState(true);
    });
  }

  pause(): void {
    this.audio.pause();
  }

  stop(): void {
    console.log('AudioPlayer.stop', { src: this.audio.src, currentTime: this.audio.currentTime });
    this.audio.pause();
    this.audio.currentTime = 0;
    this.state.positionSecs = 0;
    this.state.status = 'stopped';
    this.emitState(true);
  }

  seek(positionSecs: number): void {
    const target = Math.max(0, positionSecs);
    this.audio.currentTime = target;
    this.state.positionSecs = this.audio.currentTime;
    this.emitState(true);
  }

  setVolume(volume: number): void {
    const clamped = Math.max(0, Math.min(100, volume));
    this.state.volume = clamped;
    this.audio.volume = clamped / 100;
    this.audio.muted = clamped <= 0;
    this.emitState(true);
  }

  getState(): PlayerState {
    return {
      ...this.state,
      positionSecs: this.audio.currentTime,
    };
  }

  updateMetadata(meta: TrackMetadata | null): void {
    if (!('mediaSession' in navigator)) return;
    if (!meta) {
      navigator.mediaSession.metadata = null;
      return;
    }
    const artwork = meta.artworkUrl
      ? [{ src: meta.artworkUrl, sizes: '512x512', type: 'image/jpeg' }]
      : [];
    navigator.mediaSession.metadata = new MediaMetadata({
      title: meta.title || 'Unknown',
      artist: meta.artist || 'Unknown Artist',
      album: meta.album || '',
      artwork,
    });
  }

  destroy(): void {
    this.audio.removeEventListener('timeupdate', this.handleTimeUpdate);
    this.audio.removeEventListener('ended', this.handleEnded);
    this.audio.removeEventListener('play', this.handlePlay);
    this.audio.removeEventListener('pause', this.handlePause);
    this.audio.removeEventListener('waiting', this.handleWaiting);
    this.audio.removeEventListener('canplay', this.handleCanPlay);
    this.audio.removeEventListener('error', this.handleError);
    this.audio.pause();
    this.audio.removeAttribute('src');
    this.audio.load();
  }

  private emitState(force = false): void {
    const next: PlayerState = {
      ...this.state,
      positionSecs: this.audio.currentTime,
    };

    if (
      force ||
      next.status !== this.lastEmittedState.status ||
      next.positionSecs !== this.lastEmittedState.positionSecs ||
      next.volume !== this.lastEmittedState.volume ||
      next.currentSongIndex !== this.lastEmittedState.currentSongIndex ||
      next.projectionGeneration !== this.lastEmittedState.projectionGeneration
    ) {
      this.lastEmittedState = { ...next };
      this.state = { ...next };
      this.onStateChange(next);
    }
  }
}
