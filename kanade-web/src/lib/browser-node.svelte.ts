import type { PlayerState } from './audio-player';
import type { HlsAudioPlayer } from './hls-player';
import { buildMediaUrl } from './media-auth';
import { updateMediaBase, ws as uiWs } from './stores';

function emitWsToast(message: string) {
  window.dispatchEvent(new CustomEvent('kanade-ws-toast', { detail: { message } }));
}

type RegistrationAck = {
  node_id: string;
  media_base_url: string;
};

const BROWSER_SESSION_NODE_ID_KEY = 'kanade-browser-session-node-id';
const DEBUG_BROWSER_NODE = import.meta.env.DEV;

type NodeCommand =
  | { type: 'play' }
  | { type: 'pause' }
  | { type: 'stop' }
  | { type: 'seek'; position_secs: number }
  | { type: 'set_volume'; volume: number }
  | { type: 'set_queue'; file_paths: string[]; track_ids?: string[]; projection_generation: number }
  | { type: 'add'; file_paths: string[]; track_ids?: string[] }
  | { type: 'remove'; index: number }
  | { type: 'move_track'; from: number; to: number };

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isRegistrationAck(value: unknown): value is RegistrationAck {
  return (
    isObject(value) &&
    typeof value.node_id === 'string' &&
    typeof value.media_base_url === 'string'
  );
}

function isNodeCommand(value: unknown): value is NodeCommand {
  if (!isObject(value) || typeof value.type !== 'string') return false;

  switch (value.type) {
    case 'play':
    case 'pause':
    case 'stop':
      return true;
    case 'seek':
      return typeof value.position_secs === 'number';
    case 'set_volume':
      return typeof value.volume === 'number';
    case 'set_queue':
      return (
        Array.isArray(value.file_paths) &&
        value.file_paths.every((v) => typeof v === 'string') &&
        (value.track_ids === undefined ||
          (Array.isArray(value.track_ids) && value.track_ids.every((v) => typeof v === 'string'))) &&
        typeof value.projection_generation === 'number'
      );
    case 'add':
      return (
        Array.isArray(value.file_paths) &&
        value.file_paths.every((v) => typeof v === 'string') &&
        (value.track_ids === undefined ||
          (Array.isArray(value.track_ids) && value.track_ids.every((v) => typeof v === 'string')))
      );
    case 'remove':
      return typeof value.index === 'number';
    case 'move_track':
      return typeof value.from === 'number' && typeof value.to === 'number';
    default:
      return false;
  }
}

export class BrowserNode {
  private readonly player: HlsAudioPlayer;
  private ws: WebSocket | null = null;
  private wsUrl: string | null = null;
  private name: string | null = null;
  private logicalNodeId: string | null = null;
  private nodeId: string | null = null;
  private mediaBaseUrl: string | null = null;
  private registered = false;
  private active = false;
  private retryCount = 0;
  private reconnectTimeoutId: number | null = null;
  private connectTimeoutId: number | null = null;
  private registrationTimeoutId: number | null = null;
  private heartbeatTimeoutId: number | null = null;
  private stateIntervalId: number | null = null;
  private lastSentStateKey: string | null = null;

  connected = $state(false);

  private readonly baseDelayMs = 1000;
  private readonly maxDelayMs = 5000;
  private readonly connectTimeoutMs = 10000;
  private readonly registrationTimeoutMs = 10000;
  private readonly heartbeatTimeoutMs = 45000;

  private visibilityHandler = () => {
    if (document.visibilityState === 'visible' && !this.registered && this.active) {
      if (DEBUG_BROWSER_NODE) console.debug('BrowserNode: visibility restored, reconnecting');
      this.retryCount = 0;
      this.clearReconnectTimeout();
      this.openSocket();
    }
  };

  private onlineHandler = () => {
    if (this.active) {
      if (DEBUG_BROWSER_NODE) console.debug('BrowserNode: network online, reconnecting');
      this.retryCount = 0;
      this.clearReconnectTimeout();
      this.openSocket();
    }
  };

  private offlineHandler = () => {
    if (DEBUG_BROWSER_NODE) console.debug('BrowserNode: network offline');
    this.closeSocket();
  };

  constructor(player: HlsAudioPlayer) {
    this.player = player;
    document.addEventListener('visibilitychange', this.visibilityHandler);
    window.addEventListener('online', this.onlineHandler);
    window.addEventListener('offline', this.offlineHandler);
  }

  connect(wsUrl: string, name: string): void {
    const nextLogicalNodeId = this.getOrCreateSessionNodeId(name);

    if (
      this.active &&
      this.wsUrl === wsUrl &&
      this.name === name &&
      this.logicalNodeId === nextLogicalNodeId &&
      (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING)
    ) {
      return;
    }

    this.wsUrl = wsUrl;
    this.name = name;
    this.logicalNodeId = nextLogicalNodeId;
    this.active = true;
    this.retryCount = 0;
    this.clearReconnectTimeout();
    this.resetConnectionState();
    this.closeSocket();

    this.openSocket();
  }

  disconnect(): void {
    this.active = false;
    this.retryCount = 0;
    this.clearReconnectTimeout();
    this.resetConnectionState();
    this.closeSocket();

    if (DEBUG_BROWSER_NODE) console.debug('BrowserNode disconnected');
  }

  destroy(): void {
    this.disconnect();
    document.removeEventListener('visibilitychange', this.visibilityHandler);
    window.removeEventListener('online', this.onlineHandler);
    window.removeEventListener('offline', this.offlineHandler);
  }

  isConnected(): boolean {
    return this.registered && this.ws?.readyState === WebSocket.OPEN;
  }

  getLogicalNodeId(): string | null {
    return this.logicalNodeId;
  }

  private async signPaths(filePaths: string[], trackIds?: string[]): Promise<string[]> {
    const mediaBaseUrl = this.mediaBaseUrl;
    if (!mediaBaseUrl) {
      return filePaths;
    }

    const HLS_VARIANT = 'lossless';
    const mediaPaths = filePaths.map((filePath, i) => {
      if (filePath.startsWith('http://') || filePath.startsWith('https://')) return filePath;
      if (trackIds && trackIds[i]) {
        return `/media/hls/${trackIds[i]}/${HLS_VARIANT}/index.m3u8`;
      }
      return `/media/file/${encodeURIComponent(filePath)}`;
    });

    const signablePaths = mediaPaths.filter((path) => path.startsWith('/media/'));
    let signed = new Map<string, string>();

    if (signablePaths.length > 0) {
      try {
        signed = await uiWs.signUrls(signablePaths);
      } catch {
        signed = new Map();
      }
    }

    return mediaPaths.map((path) => {
      if (path.startsWith('http://') || path.startsWith('https://')) return path;
      return signed.get(path) ?? buildMediaUrl(mediaBaseUrl, path);
    });
  }

  private openSocket(): void {
    if (!this.wsUrl || !this.name || !this.active) return;
    if (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;

    emitWsToast('Connecting browser output…');
    if (DEBUG_BROWSER_NODE) console.debug('BrowserNode connecting');
    const ws = new WebSocket(this.wsUrl);
    this.ws = ws;
    this.registered = false;
    this.nodeId = null;
    this.mediaBaseUrl = null;

    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.clearConnectTimeout();
      if (DEBUG_BROWSER_NODE) console.debug('BrowserNode connected');
      this.sendRegistration();
      this.resetHeartbeat();
      this.clearRegistrationTimeout();
      this.registrationTimeoutId = window.setTimeout(() => {
        if (this.ws !== ws || this.registered) return;
        console.warn('BrowserNode registration ack timeout');
        ws.close();
      }, this.registrationTimeoutMs);
    };

    ws.onmessage = (event: MessageEvent<string>) => {
      if (this.ws !== ws) return;
      this.resetHeartbeat();
      this.handleMessage(event.data);
    };

    ws.onerror = () => {
      if (this.ws !== ws) return;
      console.warn('BrowserNode WebSocket error');
    };

    ws.onclose = () => {
      if (this.ws !== ws) return;
      const wasConnected = this.registered;
      this.ws = null;
      this.resetConnectionState();

      if (wasConnected) {
        if (DEBUG_BROWSER_NODE) console.debug('BrowserNode disconnected from server');
      }

      if (this.active) {
        this.scheduleReconnect();
      }
    };

    this.clearHeartbeat();
    this.clearConnectTimeout();
    this.connectTimeoutId = window.setTimeout(() => {
      if (this.ws === ws && ws.readyState === WebSocket.CONNECTING) {
        emitWsToast('Browser output timed out. Retrying…');
        console.warn('BrowserNode connect timeout');
        ws.close();
      }
    }, this.connectTimeoutMs);
  }

  private sendRegistration(): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.name || !this.logicalNodeId) return;
    try {
      this.ws.send(JSON.stringify({ node_id: this.logicalNodeId, display_name: this.name }));
      if (DEBUG_BROWSER_NODE) console.debug('BrowserNode registering');
    } catch (error) {
      console.warn('BrowserNode failed to send registration', error);
    }
  }

  private getOrCreateSessionNodeId(name: string): string {
    const existing = window.sessionStorage.getItem(BROWSER_SESSION_NODE_ID_KEY);
    if (existing) return existing;

    const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'browser';
    const uuid = typeof crypto.randomUUID === 'function'
      ? crypto.randomUUID()
      : 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
          const r = (Math.random() * 16) | 0;
          return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
        });
    const generated = `${slug}-${uuid}`;
    window.sessionStorage.setItem(BROWSER_SESSION_NODE_ID_KEY, generated);
    return generated;
  }

  private handleMessage(raw: string): void {
    let msg: unknown;

    try {
      msg = JSON.parse(raw);
    } catch (error) {
      console.warn('BrowserNode failed to parse message', error);
      return;
    }

    if (!this.registered) {
      if (!isRegistrationAck(msg)) {
        console.warn('BrowserNode invalid registration ack');
        this.ws?.close();
        return;
      }

      this.registered = true;
      this.connected = true;
      this.retryCount = 0;
      this.nodeId = msg.node_id;
      this.mediaBaseUrl = msg.media_base_url;
      updateMediaBase(msg.media_base_url);
      this.clearRegistrationTimeout();
      this.startStateUpdates();
      if (DEBUG_BROWSER_NODE) console.debug('BrowserNode registered');
      return;
    }

    if (!isNodeCommand(msg)) {
      console.warn('BrowserNode received unknown command');
      return;
    }

    this.handleCommand(msg);
  }

  private handleCommand(command: NodeCommand): void {
    void this.handleReadyCommand(command);
  }

  private async handleReadyCommand(command: NodeCommand): Promise<void> {
    if (DEBUG_BROWSER_NODE) console.debug('BrowserNode command', command.type);
    switch (command.type) {
      case 'play':
        this.safePlayerCall(() => this.player.play());
        break;
      case 'pause':
        this.safePlayerCall(() => this.player.pause());
        break;
      case 'stop':
        this.safePlayerCall(() => this.player.stop());
        break;
      case 'seek':
        this.safePlayerCall(() => this.player.seek(command.position_secs));
        break;
      case 'set_volume':
        this.safePlayerCall(() => this.player.setVolume(command.volume));
        break;
      case 'set_queue': {
        const signedPaths = await this.signPaths(command.file_paths, command.track_ids);
        this.safePlayerCall(() => this.player.setQueue(signedPaths, command.projection_generation));
        break;
      }
      case 'add': {
        const signedPaths = await this.signPaths(command.file_paths, command.track_ids);
        this.safePlayerCall(() => this.player.addTracks(signedPaths));
        break;
      }
      case 'remove':
        this.safePlayerCall(() => this.player.removeTrack(command.index));
        break;
      case 'move_track':
        this.safePlayerCall(() => this.player.moveTrack(command.from, command.to));
        break;
    }
  }

  private safePlayerCall(action: () => void | Promise<void>): void {
    try {
      const result = action();
      if (result && typeof (result as Promise<void>).then === 'function') {
        (result as Promise<void>).catch((error) => {
          console.warn('BrowserNode player command failed', error);
        });
      }
    } catch (error) {
      console.warn('BrowserNode player command failed', error);
    }
  }

  private startStateUpdates(): void {
    this.clearStateInterval();
    this.stateIntervalId = window.setInterval(() => {
      this.sendStateUpdate();
    }, 1000);
  }

  private sendStateUpdate(): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.registered) return;

    let state: PlayerState;
    try {
      state = this.player.getState();
    } catch (error) {
      console.warn('BrowserNode failed to read player state', error);
      return;
    }

    try {
      const payload = {
        status: state.status,
        position_secs: state.positionSecs,
        volume: state.volume,
        mpd_song_index: state.currentSongIndex ?? null,
        projection_generation: state.projectionGeneration,
      };
      const stateKey = JSON.stringify(payload);
      if (stateKey === this.lastSentStateKey) return;
      this.lastSentStateKey = stateKey;
      this.ws.send(stateKey);
    } catch (error) {
      console.warn('BrowserNode failed to send state update', error);
    }
  }

  private scheduleReconnect(): void {
    if (!this.active || this.reconnectTimeoutId !== null) return;

    const delay = this.retryCount === 0 ? 3000 : Math.min(this.baseDelayMs * Math.pow(2, this.retryCount), this.maxDelayMs);
    this.retryCount += 1;
    console.log(`BrowserNode reconnecting in ${delay}ms`);

    this.reconnectTimeoutId = window.setTimeout(() => {
      this.reconnectTimeoutId = null;
      this.openSocket();
    }, delay);
  }

  private clearReconnectTimeout(): void {
    if (this.reconnectTimeoutId !== null) {
      window.clearTimeout(this.reconnectTimeoutId);
      this.reconnectTimeoutId = null;
    }
  }

  private clearRegistrationTimeout(): void {
    if (this.registrationTimeoutId !== null) {
      window.clearTimeout(this.registrationTimeoutId);
      this.registrationTimeoutId = null;
    }
  }

  private clearConnectTimeout(): void {
    if (this.connectTimeoutId !== null) {
      window.clearTimeout(this.connectTimeoutId);
      this.connectTimeoutId = null;
    }
  }

  private clearStateInterval(): void {
    if (this.stateIntervalId !== null) {
      window.clearInterval(this.stateIntervalId);
      this.stateIntervalId = null;
    }
  }

  private resetHeartbeat(): void {
    this.clearHeartbeat();
    this.heartbeatTimeoutId = window.setTimeout(() => {
      console.warn('BrowserNode heartbeat timeout — no message received');
      if (this.ws) {
        this.ws.onclose = null;
        this.ws.close();
        this.ws = null;
      }
      this.resetConnectionState();
      if (this.active) {
        this.scheduleReconnect();
      }
    }, this.heartbeatTimeoutMs);
  }

  private clearHeartbeat(): void {
    if (this.heartbeatTimeoutId !== null) {
      window.clearTimeout(this.heartbeatTimeoutId);
      this.heartbeatTimeoutId = null;
    }
  }

  private resetConnectionState(): void {
    this.clearConnectTimeout();
    this.clearRegistrationTimeout();
    this.clearHeartbeat();
    this.clearStateInterval();
    this.registered = false;
    this.connected = false;
    this.nodeId = null;
    this.mediaBaseUrl = null;
    this.lastSentStateKey = null;
  }

  private closeSocket(): void {
    if (!this.ws) return;
    this.ws.onopen = null;
    this.ws.onmessage = null;
    this.ws.onclose = null;
    this.ws.onerror = null;
    this.ws.close();
    this.ws = null;
  }
}
