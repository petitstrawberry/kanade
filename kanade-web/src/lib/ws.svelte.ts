import type { ClientMessage, ServerMessage, WsCommand, WsRequest, WsResponse, Node, Track, RepeatMode } from './types';

export class WsClient {
  private ws: WebSocket | null = null;
  private url: string;
  private reqId = 0;
  private pendingRequests = new Map<number, { resolve: (val: any) => void, reject: (err: any) => void }>();
  private sendQueue: string[] = [];
  private reconnectTimeout: number | null = null;
  private maxRetries = 10;
  private retryCount = 0;

  nodes = $state<Node[]>([]);
  selectedNodeId = $state<string | null>(null);
  queue = $state<Track[]>([]);
  currentIndex = $state<number | null>(null);
  shuffle = $state(false);
  repeat = $state<RepeatMode>('off');
  connected = $state(false);

  getNodeId(): string | null {
    return this.selectedNodeId;
  }

  constructor(url: string) {
    this.url = url;
  }

  connect() {
    if (this.ws?.readyState === WebSocket.OPEN) return;
    
    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      this.connected = true;
      this.retryCount = 0;
      while (this.sendQueue.length > 0) {
        const msg = this.sendQueue.shift()!;
        this.ws!.send(msg);
      }
    };

    this.ws.onmessage = (event) => {
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

    this.ws.onclose = () => {
      this.connected = false;
      this.scheduleReconnect();
    };

    this.ws.onerror = (err) => {
      console.error('WS Error:', err);
    };
  }

  private scheduleReconnect() {
    if (this.reconnectTimeout) return;
    if (this.retryCount >= this.maxRetries) {
      console.error('WS Max retries reached');
      return;
    }
    
    const delay = Math.min(1000 * Math.pow(2, this.retryCount), 30000);
    this.retryCount++;
    console.log(`Reconnecting in ${delay}ms...`);
    
    this.reconnectTimeout = window.setTimeout(() => {
      this.reconnectTimeout = null;
      this.connect();
    }, delay);
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
