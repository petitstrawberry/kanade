import { WsClient } from './ws';
import { AudioPlayer } from './audio-player';
import { BrowserNode } from './browser-node';

const params = new URLSearchParams(window.location.search);
const wsUrl = params.get('server') || 'ws://127.0.0.1:8080';
export const mediaBase = params.get('media') || 'http://127.0.0.1:8081';

export const ws = new WsClient(wsUrl);

const g = globalThis as Record<string, unknown>;

function getBrowserNode(): BrowserNode {
  if (!g.__kanadeBrowserNode) {
    const player = new AudioPlayer(() => {});
    const node = new BrowserNode(player);
    const nodeWsUrl = wsUrl.replace(/:\d+$/, '') + ':8082';
    const nodeName = `Browser (${navigator.userAgent.includes('iPhone') ? 'iPhone' : navigator.userAgent.includes('iPad') ? 'iPad' : 'Desktop'})`;
    node.connect(nodeWsUrl, nodeName);
    g.__kanadeBrowserNode = node;
  }
  return g.__kanadeBrowserNode as BrowserNode;
}

export const browserNode = getBrowserNode();

export const selectedNodeId = $state({ value: localStorage.getItem('kanade-node-id') || '' });

export function selectNode(id: string) {
  selectedNodeId.value = id;
  localStorage.setItem('kanade-node-id', id);
}

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
