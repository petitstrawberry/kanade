export interface PlayerState {
  status: 'stopped' | 'playing' | 'paused' | 'loading';
  positionSecs: number;
  volume: number;
  currentSongIndex: number;
  projectionGeneration: number;
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
  }

  setQueue(filePaths: string[], projectionGeneration: number): void {
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
