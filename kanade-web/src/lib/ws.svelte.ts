import type { ClientMessage, ServerMessage, WsCommand, WsRequest, WsResponse, Node, Track, RepeatMode } from './types';

function emitWsToast(message: string) {
  window.dispatchEvent(new CustomEvent('kanade-ws-toast', { detail: { message } }));
}

export class WsClient {
  private ws: WebSocket | null = null;
  private url: string;
  private reqId = 0;
  private pendingRequests = new Map<number, { resolve: (val: any) => void, reject: (err: any) => void }>();
  private sendQueue: string[] = [];
  private reconnectTimeout: number | null = null;
  private connectTimeout: number | null = null;
  private heartbeatTimeout: number | null = null;
  private retryCount = 0;
  private active = false;
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

  setFallbackUrl(url: string) {
    this.fallbackUrl = url;
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
            const variantData = Object.values(msg.data)[0];
            req.resolve(variantData);
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
      if (this.pendingRequests.size > 0) {
        const error = new Error('Disconnected');
        for (const [id, req] of this.pendingRequests.entries()) {
          req.reject(error);
          this.pendingRequests.delete(id);
        }
      }
      if (this.active) this.scheduleReconnect();
    };

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
      if (this.pendingRequests.size > 0) {
        const error = new Error('Heartbeat timeout');
        for (const [id, req] of this.pendingRequests.entries()) {
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
      this.pendingRequests.set(id, { resolve, reject });
      
      const msg: ClientMessage = { ...req, req_id: id };
      this.sendRaw(JSON.stringify(msg));

      setTimeout(() => {
        if (this.pendingRequests.has(id)) {
          this.pendingRequests.delete(id);
          reject(new Error('Request timed out'));
        }
      }, 10000);
    }) as Promise<WsResponse>;
  }
}
