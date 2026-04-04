import { AudioPlayer } from './audio-player';
import type { PlayerState } from './audio-player';

type RegistrationAck = {
  node_id: string;
  media_base_url: string;
};

const BROWSER_SESSION_NODE_ID_KEY = 'kanade-browser-session-node-id';

type NodeCommand =
  | { type: 'play' }
  | { type: 'pause' }
  | { type: 'stop' }
  | { type: 'seek'; position_secs: number }
  | { type: 'set_volume'; volume: number }
  | { type: 'set_queue'; file_paths: string[]; projection_generation: number }
  | { type: 'add'; file_paths: string[] }
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
        typeof value.projection_generation === 'number'
      );
    case 'add':
      return Array.isArray(value.file_paths) && value.file_paths.every((v) => typeof v === 'string');
    case 'remove':
      return typeof value.index === 'number';
    case 'move_track':
      return typeof value.from === 'number' && typeof value.to === 'number';
    default:
      return false;
  }
}

export class BrowserNode {
  private readonly player: AudioPlayer;
  private ws: WebSocket | null = null;
  private wsUrl: string | null = null;
  private name: string | null = null;
  private logicalNodeId: string | null = null;
  private nodeId: string | null = null;
  private mediaBaseUrl: string | null = null;
  private registered = false;
  private shouldReconnect = false;
  private retryCount = 0;
  private reconnectTimeoutId: number | null = null;
  private registrationTimeoutId: number | null = null;
  private stateIntervalId: number | null = null;

  private readonly maxRetries = 5;
  private readonly baseDelayMs = 1000;
  private readonly maxDelayMs = 15000;
  private readonly registrationTimeoutMs = 5000;

  constructor(player: AudioPlayer) {
    this.player = player;
  }

  connect(wsUrl: string, name: string): void {
    this.wsUrl = wsUrl;
    this.name = name;
    this.logicalNodeId = this.getOrCreateSessionNodeId(name);
    this.shouldReconnect = true;
    this.retryCount = 0;
    this.clearReconnectTimeout();
    this.clearStateInterval();
    this.clearRegistrationTimeout();

    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onmessage = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.close();
      this.ws = null;
    }

    this.openSocket();
  }

  disconnect(): void {
    this.shouldReconnect = false;
    this.retryCount = 0;
    this.registered = false;
    this.nodeId = null;
    this.mediaBaseUrl = null;
    this.clearReconnectTimeout();
    this.clearStateInterval();
    this.clearRegistrationTimeout();

    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onmessage = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.close();
      this.ws = null;
    }

    console.log('BrowserNode disconnected');
  }

  isConnected(): boolean {
    return this.registered && this.ws?.readyState === WebSocket.OPEN;
  }

  getLogicalNodeId(): string | null {
    return this.logicalNodeId;
  }

  private toHttpUrl(filePath: string): string {
    if (filePath.startsWith('http://') || filePath.startsWith('https://')) return filePath;
    return `${this.mediaBaseUrl}/media/file/${encodeURIComponent(filePath)}`;
  }

  private openSocket(): void {
    if (!this.wsUrl || !this.name || !this.shouldReconnect) return;

    console.log(`BrowserNode connecting: ${this.wsUrl}`);
    const ws = new WebSocket(this.wsUrl);
    this.ws = ws;
    this.registered = false;
    this.nodeId = null;
    this.mediaBaseUrl = null;

    ws.onopen = () => {
      if (this.ws !== ws) return;
      console.log(`BrowserNode connected: ${this.wsUrl}`);
      this.sendRegistration();
      this.clearRegistrationTimeout();
      this.registrationTimeoutId = window.setTimeout(() => {
        if (this.ws !== ws || this.registered) return;
        console.warn('BrowserNode registration ack timeout');
        ws.close();
      }, this.registrationTimeoutMs);
    };

    ws.onmessage = (event: MessageEvent<string>) => {
      if (this.ws !== ws) return;
      this.handleMessage(event.data);
    };

    ws.onerror = () => {
      if (this.ws !== ws) return;
      console.warn('BrowserNode WebSocket error');
      ws.close();
    };

    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      const wasConnected = this.registered;
      this.registered = false;
      this.nodeId = null;
      this.mediaBaseUrl = null;
      this.clearRegistrationTimeout();
      this.clearStateInterval();

      if (wasConnected) {
        console.log('BrowserNode disconnected from server');
      }

      if (this.shouldReconnect) {
        this.scheduleReconnect();
      }
    };
  }

  private sendRegistration(): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN || !this.name || !this.logicalNodeId) return;
    try {
      this.ws.send(JSON.stringify({ node_id: this.logicalNodeId, display_name: this.name }));
      console.log(`BrowserNode registering: ${this.logicalNodeId} (${this.name})`);
    } catch (error) {
      console.warn('BrowserNode failed to send registration', error);
    }
  }

  private getOrCreateSessionNodeId(name: string): string {
    const existing = window.sessionStorage.getItem(BROWSER_SESSION_NODE_ID_KEY);
    if (existing) return existing;

    const generated = `${name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'browser'}-${crypto.randomUUID()}`;
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
      this.retryCount = 0;
      this.nodeId = msg.node_id;
      this.mediaBaseUrl = msg.media_base_url;
      this.clearRegistrationTimeout();
      this.startStateUpdates();
      console.log(`BrowserNode registered: ${this.nodeId}`);
      console.log(`BrowserNode media base URL: ${this.mediaBaseUrl}`);
      return;
    }

    if (!isNodeCommand(msg)) {
      console.warn('BrowserNode received unknown command');
      return;
    }

    this.handleCommand(msg);
  }

  private handleCommand(command: NodeCommand): void {
    console.log('BrowserNode command', command);
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
      case 'set_queue':
        this.safePlayerCall(() => this.player.setQueue(command.file_paths.map(p => this.toHttpUrl(p)), command.projection_generation));
        break;
      case 'add':
        this.safePlayerCall(() => this.player.addTracks(command.file_paths.map(p => this.toHttpUrl(p))));
        break;
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
      this.ws.send(
        JSON.stringify({
          status: state.status,
          position_secs: state.positionSecs,
          volume: state.volume,
          mpd_song_index: state.currentSongIndex ?? null,
          projection_generation: state.projectionGeneration,
        }),
      );
    } catch (error) {
      console.warn('BrowserNode failed to send state update', error);
    }
  }

  private scheduleReconnect(): void {
    if (!this.shouldReconnect || this.reconnectTimeoutId !== null) return;

    if (this.retryCount >= this.maxRetries) {
      console.warn('BrowserNode max reconnect retries reached');
      return;
    }

    const delay = Math.min(this.baseDelayMs * Math.pow(2, this.retryCount), this.maxDelayMs);
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

  private clearStateInterval(): void {
    if (this.stateIntervalId !== null) {
      window.clearInterval(this.stateIntervalId);
      this.stateIntervalId = null;
    }
  }
}
