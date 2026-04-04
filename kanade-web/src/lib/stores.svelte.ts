import { WsClient } from './ws';
import { AudioPlayer } from './audio-player';
import { BrowserNode } from './browser-node';

const params = new URLSearchParams(window.location.search);
const host = location.hostname;
const wsUrl = params.get('server') || `ws://${host}:8080`;
export const mediaBase = params.get('media') || `http://${host}:8081`;

export const ws = new WsClient(wsUrl);

const g = globalThis as Record<string, unknown>;

function getBrowserNode(): BrowserNode {
  if (!g.__kanadeBrowserNode) {
    const player = new AudioPlayer(() => {});
    const node = new BrowserNode(player);
    const nodeWs = new URL(wsUrl);
    nodeWs.port = '8082';
    nodeWs.pathname = '/';
    nodeWs.search = '';
    nodeWs.hash = '';
    const nodeWsUrl = nodeWs.toString();
    const nodeName = `Browser (${navigator.userAgent.includes('iPhone') ? 'iPhone' : navigator.userAgent.includes('iPad') ? 'iPad' : 'Desktop'})`;
    node.connect(nodeWsUrl, nodeName);
    g.__kanadeBrowserNode = node;
  }
  return g.__kanadeBrowserNode as BrowserNode;
}

export const browserNode = getBrowserNode();

export class ActiveTab {
  value = $state<'library' | 'queue' | 'search'>('library');
}

export type Toast = { message: string; id: number };
let toastId = 0;
export const toasts = $state<Toast[]>([]);

export function showToast(message: string) {
  const id = ++toastId;
  toasts.push({ message, id });
  setTimeout(() => {
    toasts.splice(toasts.findIndex(t => t.id === id), 1);
  }, 2000);
}
