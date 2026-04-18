import type { ClientMessage, ServerMessage, WsCommand, WsRequest, Node, Track, RepeatMode } from './types';

function emitWsToast(message: string) {
  window.dispatchEvent(new CustomEvent('kanade-ws-toast', { detail: { message } }));
}

type ResponseMessage = Extract<ServerMessage, { type: 'response' }>;

type PendingRequest = {
  resolve: (val: ResponseMessage) => void;
  reject: (err: unknown) => void;
  timeoutId: number;
};

type SignedUrlCacheEntry = {
  url: string;
  expiresAt: number;
};

type PendingSignBatch = {
  timerId: number;
  paths: Set<string>;
  resolvers: Array<{
    resolve: (value: Map<string, string>) => void;
    reject: (reason?: unknown) => void;
  }>;
};

const SIGNED_URL_TTL_MS = 15 * 60 * 1000;
const SIGN_URL_BATCH_WINDOW_MS = 50;

export class WsClient {
  private ws: WebSocket | null = null;
  private url: string;
  private reqId = 0;
  private pendingRequests = new Map<number, PendingRequest>();
  private sendQueue: string[] = [];
  private reconnectTimeout: number | null = null;
  private connectTimeout: number | null = null;
  private heartbeatTimeout: number | null = null;
  private retryCount = 0;
  private active = false;
  private signedUrlCache = new Map<string, SignedUrlCacheEntry>();
  private pendingSignBatch: PendingSignBatch | null = null;
  private readonly connectTimeoutMs = 5000;
  private readonly heartbeatTimeoutMs = 45000;

  nodes = $state<Node[]>([]);
  selectedNodeId = $state<string | null>(null);
  queue = $state<Track[]>([]);
  currentIndex = $state<number | null>(null);
  shuffle = $state(false);
  repeat = $state<RepeatMode>('off');
  connected = $state(false);

  private visibilityHandler = () => {
    if (document.visibilityState === 'visible' && !this.connected && this.active) {
      console.log('WS: visibility restored, reconnecting');
      this.retryCount = 0;
      this.clearReconnectTimeout();
      this.connect();
    }
  };

  private onlineHandler = () => {
    if (this.active) this.scheduleReconnect();
  };

  private offlineHandler = () => {
    console.log('WS: network offline');
    this.cancelPendingSignBatch(new Error('Disconnected'));
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = null;
    }
  };

  getNodeId(): string | null {
    return this.selectedNodeId;
  }

  constructor(url: string) {
    this.url = url;
    document.addEventListener('visibilitychange', this.visibilityHandler);
    window.addEventListener('online', this.onlineHandler);
    window.addEventListener('offline', this.offlineHandler);
  }

  reconnectTo(url: string): void {
    this.url = url;
    this.reqId = 0;
    this.sendQueue.length = 0;
    this.nodes = [];
    this.selectedNodeId = null;
    this.queue = [];
    this.currentIndex = null;
    this.shuffle = false;
    this.repeat = 'off';
    this.signedUrlCache.clear();
    this.cancelPendingSignBatch(new Error('Disconnected'));
    this.disconnect();
    this.connect();
  }

  connect() {
    this.active = true;
    this.clearReconnectTimeout();
    if (this.ws?.readyState === WebSocket.OPEN || this.ws?.readyState === WebSocket.CONNECTING) return;

    emitWsToast('Connecting to server…');
    const ws = new WebSocket(this.url);
    this.ws = ws;

    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.clearConnectTimeout();
      while (this.sendQueue.length > 0) {
        const msg = this.sendQueue.shift()!;
        ws.send(msg);
      }
      this.connected = true;
      this.retryCount = 0;
      this.resetHeartbeat();
    };

    ws.onmessage = (event) => {
      if (this.ws !== ws) return;
      this.resetHeartbeat();
      try {
        const msg: ServerMessage = JSON.parse(event.data);
        if (msg.type === 'state') {
          this.nodes = msg.state.nodes;
          this.selectedNodeId = msg.state.selected_node_id;
          this.queue = msg.state.queue;
          this.currentIndex = msg.state.current_index;
          this.shuffle = msg.state.shuffle;
          this.repeat = msg.state.repeat;
        } else if (msg.type === 'response') {
          const req = this.pendingRequests.get(msg.req_id);
          if (req) {
            window.clearTimeout(req.timeoutId);
            req.resolve(msg);
            this.pendingRequests.delete(msg.req_id);
          }
        }
      } catch (err) {
        console.error('Failed to parse WS message:', err);
      }
    };

    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.clearConnectTimeout();
      this.clearHeartbeat();
      this.ws = null;
      this.connected = false;
      this.sendQueue.length = 0;
      this.cancelPendingSignBatch(new Error('Disconnected'));
      if (this.pendingRequests.size > 0) {
        const error = new Error('Disconnected');
        for (const [id, req] of this.pendingRequests.entries()) {
          window.clearTimeout(req.timeoutId);
          req.reject(error);
          this.pendingRequests.delete(id);
        }
      }
      if (this.active) this.scheduleReconnect();
    };

    ws.onerror = (err) => {
      if (this.ws !== ws) return;
      this.clearConnectTimeout();
      console.error('WS Error:', err);
    };

    this.clearConnectTimeout();
    this.connectTimeout = window.setTimeout(() => {
      if (this.ws === ws && ws.readyState === WebSocket.CONNECTING) {
        emitWsToast('Server connection timed out. Retrying…');
        console.warn('WS connect timeout');
        ws.close();
      }
    }, this.connectTimeoutMs);
  }

  private scheduleReconnect() {
    if (!this.active) return;
    if (this.reconnectTimeout) return;

    const delay = this.retryCount === 0 ? 3000 : Math.min(1000 * Math.pow(2, this.retryCount), 5000);
    this.retryCount++;
    console.log(`Reconnecting in ${delay}ms...`);

    this.reconnectTimeout = window.setTimeout(() => {
      this.reconnectTimeout = null;
      this.connect();
    }, delay);
  }

  private clearReconnectTimeout() {
    if (this.reconnectTimeout !== null) {
      window.clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
  }

  disconnect() {
    this.active = false;
    this.connected = false;
    this.retryCount = 0;
    this.clearConnectTimeout();
    this.clearReconnectTimeout();
    this.clearHeartbeat();
    this.cancelPendingSignBatch(new Error('Disconnected'));
    if (this.ws) {
      this.ws.onopen = null;
      this.ws.onmessage = null;
      this.ws.onclose = null;
      this.ws.onerror = null;
      this.ws.close();
      this.ws = null;
    }

    if (this.pendingRequests.size > 0) {
      const error = new Error('Disconnected');
      for (const [id, req] of this.pendingRequests.entries()) {
        window.clearTimeout(req.timeoutId);
        req.reject(error);
        this.pendingRequests.delete(id);
      }
    }
  }

  private clearConnectTimeout() {
    if (this.connectTimeout !== null) {
      window.clearTimeout(this.connectTimeout);
      this.connectTimeout = null;
    }
  }

  private resetHeartbeat() {
    this.clearHeartbeat();
    this.heartbeatTimeout = window.setTimeout(() => {
      console.warn('WS heartbeat timeout — no message received');
      if (this.ws) {
        this.ws.onclose = null;
        this.ws.close();
        this.ws = null;
      }
      this.connected = false;
      this.sendQueue.length = 0;
      this.cancelPendingSignBatch(new Error('Heartbeat timeout'));
      if (this.pendingRequests.size > 0) {
        const error = new Error('Heartbeat timeout');
        for (const [id, req] of this.pendingRequests.entries()) {
          window.clearTimeout(req.timeoutId);
          req.reject(error);
          this.pendingRequests.delete(id);
        }
      }
      if (this.active) this.scheduleReconnect();
    }, this.heartbeatTimeoutMs);
  }

  private clearHeartbeat() {
    if (this.heartbeatTimeout !== null) {
      window.clearTimeout(this.heartbeatTimeout);
      this.heartbeatTimeout = null;
    }
  }

  private sendRaw(json: string) {
    if (this.connected && this.ws) {
      this.ws.send(json);
    } else {
      this.sendQueue.push(json);
    }
  }

  sendCommand(cmd: WsCommand) {
    const msg: ClientMessage = cmd;
    console.log('ws.sendCommand', msg, { connected: this.connected });
    this.sendRaw(JSON.stringify(msg));
  }

  sendRequest(req: WsRequest): Promise<any> {
    return new Promise((resolve, reject) => {
      const id = ++this.reqId;
      const timeoutId = window.setTimeout(() => {
        if (this.pendingRequests.has(id)) {
          this.pendingRequests.delete(id);
          reject(new Error('Request timed out'));
        }
      }, 10000);

      this.pendingRequests.set(id, {
        resolve: (message) => resolve(message.data),
        reject,
        timeoutId,
      });

      const msg: ClientMessage = { ...req, req_id: id };
      this.sendRaw(JSON.stringify(msg));
    });
  }

  signUrls(paths: string[]): Promise<Map<string, string>> {
    const uniquePaths = Array.from(new Set(paths.filter(Boolean)));
    if (uniquePaths.length === 0) {
      return Promise.resolve(new Map());
    }

    const now = Date.now();
    const resolved = new Map<string, string>();
    const missing: string[] = [];

    for (const path of uniquePaths) {
      const cached = this.signedUrlCache.get(path);
      if (cached && cached.expiresAt > now) {
        resolved.set(path, cached.url);
      } else {
        if (cached) this.signedUrlCache.delete(path);
        missing.push(path);
      }
    }

    if (missing.length === 0) {
      return Promise.resolve(resolved);
    }

    return new Promise((resolve, reject) => {
      const batch = this.pendingSignBatch ?? this.createPendingSignBatch();
      for (const path of missing) {
        batch.paths.add(path);
      }
      batch.resolvers.push({
        resolve: (signedMap) => {
          const merged = new Map<string, string>();
          for (const path of uniquePaths) {
            const signedUrl = resolved.get(path) ?? signedMap.get(path);
            if (signedUrl) {
              merged.set(path, signedUrl);
            }
          }
          resolve(merged);
        },
        reject,
      });
    });
  }

  private createPendingSignBatch(): PendingSignBatch {
    const batch: PendingSignBatch = {
      timerId: window.setTimeout(() => {
        void this.flushPendingSignBatch();
      }, SIGN_URL_BATCH_WINDOW_MS),
      paths: new Set(),
      resolvers: [],
    };
    this.pendingSignBatch = batch;
    return batch;
  }

  private async flushPendingSignBatch(): Promise<void> {
    const batch = this.pendingSignBatch;
    if (!batch) return;
    this.pendingSignBatch = null;
    window.clearTimeout(batch.timerId);

    const paths = Array.from(batch.paths);
    if (paths.length === 0) {
      for (const resolver of batch.resolvers) {
        resolver.resolve(new Map());
      }
      return;
    }

    try {
      const response = await this.sendRequest({ req: 'sign_urls', paths });
      const signedUrlObject = response?.signed_urls ?? {};
      const now = Date.now();
      const signedMap = new Map<string, string>();

      for (const [path, url] of Object.entries(signedUrlObject)) {
        signedMap.set(path, url);
        this.signedUrlCache.set(path, { url, expiresAt: now + SIGNED_URL_TTL_MS });
      }

      for (const resolver of batch.resolvers) {
        resolver.resolve(signedMap);
      }
    } catch (error) {
      for (const resolver of batch.resolvers) {
        resolver.reject(error);
      }
    }
  }

  private cancelPendingSignBatch(error: Error): void {
    const batch = this.pendingSignBatch;
    if (!batch) return;
    this.pendingSignBatch = null;
    window.clearTimeout(batch.timerId);
    for (const resolver of batch.resolvers) {
      resolver.reject(error);
    }
  }
}
