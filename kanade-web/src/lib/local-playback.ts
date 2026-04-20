import type { PlayerState } from './audio-player';
import { HlsAudioPlayer } from './hls-player';
import type { PlaybackStatus, RepeatMode, Track } from './types';

export interface LocalPlaybackState {
  queue: Track[];
  currentIndex: number | null;
  currentTrack: Track | null;
  status: PlaybackStatus;
  positionSecs: number;
  durationSecs: number;
  volume: number;
  repeatMode: RepeatMode;
  shuffleEnabled: boolean;
}

export type PlaybackStateCallback = (state: LocalPlaybackState) => void;

type SyncablePlayerState = PlayerState & { durationSecs?: number };

export class LocalPlaybackController {
  private static readonly HLS_VARIANT = 'lossless';

  private readonly player: HlsAudioPlayer;
  private readonly signUrls: (paths: string[]) => Promise<Map<string, string>>;
  private readonly onStateChange: PlaybackStateCallback;

  private tracks: Track[] = [];
  private _currentIndex: number | null = null;
  private _repeatMode: RepeatMode = 'off';
  private _shuffleEnabled = false;
  private shuffleOrder: number[] = [];

  private _status: PlaybackStatus = 'stopped';
  private _positionSecs = 0;
  private _durationSecs = 0;
  private _volume = 100;

  private lastStateKey = '';
  private loadAbortController: AbortController | null = null;
  private preloadAbortController: AbortController | null = null;
  private preloadedNextIndex: number | null = null;

  constructor(
    player: HlsAudioPlayer,
    signUrls: (paths: string[]) => Promise<Map<string, string>>,
    onStateChange: PlaybackStateCallback,
  ) {
    this.player = player;
    this.signUrls = signUrls;
    this.onStateChange = onStateChange;
    this.player.onPlaybackFinished = () => {
      this.handlePlaybackFinished();
    };
    this.player.onTrackAdvanced = () => {
      this.handleTrackAdvanced();
    };
  }

  get queue(): Track[] {
    return this.tracks;
  }

  get currentIndex(): number | null {
    return this._currentIndex;
  }

  get currentTrack(): Track | null {
    if (this._currentIndex === null) return null;
    return this.tracks[this._currentIndex] ?? null;
  }

  get status(): PlaybackStatus {
    return this._status;
  }

  get positionSecs(): number {
    return this._positionSecs;
  }

  get durationSecs(): number {
    return this._durationSecs;
  }

  get volume(): number {
    return this._volume;
  }

  get repeatMode(): RepeatMode {
    return this._repeatMode;
  }

  get shuffleEnabled(): boolean {
    return this._shuffleEnabled;
  }

  get state(): LocalPlaybackState {
    return {
      queue: this.tracks,
      currentIndex: this._currentIndex,
      currentTrack: this.currentTrack,
      status: this._status,
      positionSecs: this._positionSecs,
      durationSecs: Math.max(this._durationSecs, this.currentTrack?.duration_secs ?? 0),
      volume: this._volume,
      repeatMode: this._repeatMode,
      shuffleEnabled: this._shuffleEnabled,
    };
  }

  playTracks(tracks: Track[], startIndex = 0): void {
    this.tracks = [...tracks];
    this._currentIndex = this.clampIndex(startIndex, this.tracks.length);
    this.buildShuffleOrder();
    void this.loadCurrentTrack(true);
  }

  addToQueue(track: Track): void {
    this.tracks.push(track);
    this.buildShuffleOrder();
    if (this.currentTrack === null && this.tracks.length === 1) {
      this._currentIndex = 0;
      void this.loadCurrentTrack(false);
      return;
    }
    this.emitState();
  }

  addTracksToQueue(tracks: Track[]): void {
    const hadCurrentTrack = this.currentTrack !== null;
    this.tracks.push(...tracks);
    this.buildShuffleOrder();
    if (!hadCurrentTrack && this.tracks.length > 0) {
      this._currentIndex = 0;
      void this.loadCurrentTrack(false);
      return;
    }
    this.emitState();
  }

  removeFromQueue(index: number): void {
    if (index < 0 || index >= this.tracks.length) return;

    const previousTrackId = this.currentTrack?.id;
    const shouldAutoplay = this._status === 'playing' || this._status === 'loading';

    this.tracks.splice(index, 1);

    if (this._currentIndex !== null) {
      if (index < this._currentIndex) {
        this._currentIndex -= 1;
      } else if (index === this._currentIndex) {
        if (this.tracks.length === 0) {
          this._currentIndex = null;
          this.buildShuffleOrder();
          this.stop();
          return;
        }

        if (this._currentIndex >= this.tracks.length) {
          this._currentIndex = this.tracks.length - 1;
        }
      }
    }

    this.buildShuffleOrder();

    if (this.currentTrack?.id !== previousTrackId) {
      void this.loadCurrentTrack(shouldAutoplay);
      return;
    }

    this.emitState();
  }

  moveInQueue(from: number, to: number): void {
    if (from === to || from < 0 || to < 0 || from >= this.tracks.length || to >= this.tracks.length) {
      return;
    }

    const [track] = this.tracks.splice(from, 1);
    if (!track) return;
    this.tracks.splice(to, 0, track);

    if (this._currentIndex !== null) {
      if (this._currentIndex === from) {
        this._currentIndex = to;
      } else if (from < this._currentIndex && to >= this._currentIndex) {
        this._currentIndex -= 1;
      } else if (from > this._currentIndex && to <= this._currentIndex) {
        this._currentIndex += 1;
      }
    }

    this.buildShuffleOrder();
    this.emitState();
  }

  clearQueue(): void {
    this.tracks = [];
    this._currentIndex = null;
    this.shuffleOrder = [];
    this.stop();
  }

  jumpToIndex(index: number): void {
    if (index < 0 || index >= this.tracks.length) return;
    this._currentIndex = index;
    this.buildShuffleOrder();
    void this.loadCurrentTrack(true);
  }

  play(): void {
    if (this.tracks.length === 0) return;

    if (this.currentTrack === null) {
      this._currentIndex = 0;
      this.buildShuffleOrder();
      void this.loadCurrentTrack(true);
      return;
    }

    if (this._status === 'stopped') {
      void this.loadCurrentTrack(true);
      return;
    }

    this.player.play();
  }

  pause(): void {
    this.player.pause();
  }

  stop(): void {
    this.loadAbortController?.abort();
    this.loadAbortController = null;
    this.clearPreload();
    this.player.stop();
    this._status = 'stopped';
    this._positionSecs = 0;
    this.emitState();
  }

  seek(positionSecs: number): void {
    this.player.seek(positionSecs);
  }

  setVolume(volume: number): void {
    this._volume = Math.max(0, Math.min(100, volume));
    this.player.setVolume(this._volume);
    this.emitState();
  }

  next(): void {
    if (!this.advance()) {
      this.stop();
      return;
    }
    this.buildShuffleOrder();
    void this.loadCurrentTrack(true);
  }

  previous(): void {
    if (this._positionSecs > 3) {
      this.seek(0);
      return;
    }

    if (!this.goBack()) {
      this.seek(0);
      return;
    }

    this.buildShuffleOrder();
    void this.loadCurrentTrack(true);
  }

  setRepeat(mode: RepeatMode): void {
    this._repeatMode = mode;
    this.updatePreloadIfNeeded();
    this.emitState();
  }

  setShuffle(enabled: boolean): void {
    this._shuffleEnabled = enabled;
    this.buildShuffleOrder();
    this.updatePreloadIfNeeded();
    this.emitState();
  }

  syncFromPlayerState(playerState: SyncablePlayerState): void {
    this._positionSecs = playerState.positionSecs;
    this._durationSecs = playerState.durationSecs ?? this._durationSecs;
    this._volume = playerState.volume;

    switch (playerState.status) {
      case 'playing':
        this._status = 'playing';
        this.updatePreloadIfNeeded();
        break;
      case 'paused':
        this._status = 'paused';
        break;
      case 'stopped':
        this._status = 'stopped';
        break;
      case 'loading':
        this._status = 'loading';
        break;
    }

    this.emitState();
  }

  destroy(): void {
    this.loadAbortController?.abort();
    this.clearPreload();
    this.player.onPlaybackFinished = null;
    this.player.onTrackAdvanced = null;
    this.player.destroy();
  }

  private async loadCurrentTrack(autoplay: boolean): Promise<void> {
    const track = this.currentTrack;
    if (!track) {
      this.stop();
      return;
    }

    this.loadAbortController?.abort();
    this.clearPreload();

    const abortController = new AbortController();
    this.loadAbortController = abortController;
    const trackId = track.id;

    this._status = autoplay ? 'loading' : 'paused';
    this._positionSecs = 0;
    this._durationSecs = Math.max(track.duration_secs ?? 0, 0);
    this.player.updateMetadata({
      title: track.title || track.file_path.split('/').pop() || 'Unknown',
      artist: track.artist || 'Unknown Artist',
      album: track.album_title || '',
      artworkUrl: null,
    });
    this.emitState();

    try {
      const hlsPath = this.hlsPathForTrack(trackId);
      const signed = await this.signUrls([hlsPath]);

      if (abortController.signal.aborted) return;
      if (this.currentTrack?.id !== trackId) return;

      const signedUrl = signed.get(hlsPath);
      if (!signedUrl) throw new Error('Failed to sign HLS URL');

      this.player.setQueue([signedUrl], 0);
      if (autoplay) {
        this.player.play();
      }
    } catch (error) {
      if (abortController.signal.aborted) return;
      if (this.currentTrack?.id !== trackId) return;
      console.error('Failed to load track:', error);
      this._status = 'stopped';
      this.emitState();
    }
  }

  private handlePlaybackFinished(): void {
    if (!this.advance()) {
      this.stop();
      return;
    }

    this.buildShuffleOrder();
    void this.loadCurrentTrack(true);
  }

  private handleTrackAdvanced(): void {
    if (!this.advance()) {
      this.stop();
      return;
    }

    this.buildShuffleOrder();
    this.preloadedNextIndex = null;
    this._positionSecs = 0;
    this._durationSecs = Math.max(this.currentTrack?.duration_secs ?? 0, 0);
    this._status = 'loading';
    this.emitState();
    this.updatePreloadIfNeeded();
  }

  private advance(): boolean {
    if (this._currentIndex === null) return false;

    if (this._repeatMode === 'one') {
      return true;
    }

    const nextIndex = this.resolveNextIndex();
    if (nextIndex === null) {
      return false;
    }

    this._currentIndex = nextIndex;
    return true;
  }

  private goBack(): boolean {
    if (this._currentIndex === null) return false;

    if (this._shuffleEnabled) {
      const shufflePosition = this.shuffleOrder.indexOf(this._currentIndex);
      if (shufflePosition <= 0) return false;
      this._currentIndex = this.shuffleOrder[shufflePosition - 1] ?? null;
      return this._currentIndex !== null;
    }

    if (this._currentIndex <= 0) return false;
    this._currentIndex -= 1;
    return true;
  }

  private resolveNextIndex(): number | null {
    if (this._currentIndex === null) return null;

    if (this._shuffleEnabled) {
      const shufflePosition = this.shuffleOrder.indexOf(this._currentIndex);
      const nextShuffleIndex = this.shuffleOrder[shufflePosition + 1];
      if (nextShuffleIndex !== undefined) {
        return nextShuffleIndex;
      }
      if (this._repeatMode === 'all') {
        return this.shuffleOrder[0] ?? null;
      }
      return null;
    }

    const nextIndex = this._currentIndex + 1;
    if (nextIndex < this.tracks.length) {
      return nextIndex;
    }

    if (this._repeatMode === 'all') {
      return this.tracks.length > 0 ? 0 : null;
    }

    return null;
  }

  private updatePreloadIfNeeded(): void {
    if (this._status !== 'playing') {
      this.clearPreload();
      return;
    }

    const nextIndex = this.resolveNextIndex();
    if (nextIndex === null || nextIndex === this._currentIndex) {
      this.clearPreload();
      return;
    }

    if (this.preloadedNextIndex === nextIndex) {
      return;
    }

    const nextTrack = this.tracks[nextIndex];
    if (!nextTrack) {
      this.clearPreload();
      return;
    }

    this.preloadAbortController?.abort();
    this.player.preloadNext(null);
    this.preloadedNextIndex = null;

    const abortController = new AbortController();
    this.preloadAbortController = abortController;
    const expectedTrackId = this.currentTrack?.id;
    const nextTrackId = nextTrack.id;
    const hlsPath = this.hlsPathForTrack(nextTrackId);

    void this.signUrls([hlsPath]).then((signed) => {
      if (abortController.signal.aborted) return;
      if (this.currentTrack?.id !== expectedTrackId) return;
      if (this.resolveNextIndex() !== nextIndex) return;

      const signedUrl = signed.get(hlsPath);
      if (!signedUrl) return;

      this.preloadedNextIndex = nextIndex;
      this.player.preloadNext(signedUrl);
    }).catch((error) => {
      if (abortController.signal.aborted) return;
      console.warn('Failed to preload next track:', error);
    });
  }

  private clearPreload(): void {
    this.preloadAbortController?.abort();
    this.preloadAbortController = null;
    this.preloadedNextIndex = null;
    this.player.preloadNext(null);
  }

  private buildShuffleOrder(): void {
    if (!this._shuffleEnabled || this.tracks.length === 0) {
      this.shuffleOrder = [];
      return;
    }

    const current = this._currentIndex ?? 0;
    const indices = Array.from({ length: this.tracks.length }, (_, index) => index);
    const currentPosition = indices.indexOf(current);

    if (currentPosition > 0) {
      [indices[0], indices[currentPosition]] = [indices[currentPosition], indices[0]];
    }

    for (let index = indices.length - 1; index > 1; index -= 1) {
      const swapIndex = 1 + Math.floor(Math.random() * index);
      [indices[index], indices[swapIndex]] = [indices[swapIndex], indices[index]];
    }

    this.shuffleOrder = indices;
  }

  private hlsPathForTrack(trackId: string): string {
    return `/media/hls/${trackId}/${LocalPlaybackController.HLS_VARIANT}/index.m3u8`;
  }

  private clampIndex(index: number, length: number): number | null {
    if (length <= 0) return null;
    return Math.max(0, Math.min(index, length - 1));
  }

  private emitState(): void {
    const state = this.state;
    const key = JSON.stringify(state);
    if (key === this.lastStateKey) return;
    this.lastStateKey = key;
    this.onStateChange(state);
  }
}
