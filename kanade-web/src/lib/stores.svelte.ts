import { WsClient } from './ws';
import { AudioPlayer } from './audio-player';
import { BrowserNode } from './browser-node';

const params = new URLSearchParams(window.location.search);
const wsScheme = location.protocol === 'https:' ? 'wss' : 'ws';
const httpScheme = location.protocol === 'https:' ? 'https' : 'http';
const host = location.hostname;

function normalizeUrl(raw: string | null, fallback: string, scheme: string): string {
  if (!raw) return fallback;
  if (/^[a-zA-Z][a-zA-Z\d+.-]*:\/\//.test(raw)) return raw;
  return `${scheme}://${raw}`;
}

function buildWsUrl(raw: string | null, fallback: string): string {
  const base = normalizeUrl(raw, fallback, wsScheme);
  if (base.endsWith('/ws')) return base;
  return base + '/ws';
}

const wsUrl = buildWsUrl(params.get('server'), `${wsScheme}://${host}:8080/ws`);
export const mediaBase = normalizeUrl(params.get('media'), `${httpScheme}://${host}:8080`, httpScheme);

export const ws = new WsClient(wsUrl);

const player = new AudioPlayer(() => {});
export function getPlayer(): AudioPlayer {
  return player;
}

export const browserNode = new BrowserNode(player);

export function connectBrowserNode(): void {
  const nodeWs = new URL(wsUrl);
  nodeWs.pathname = '/ws';
  nodeWs.search = '';
  nodeWs.hash = '';
  const nodeWsUrl = nodeWs.toString();
  const nodeName = `Browser (${navigator.userAgent.includes('iPhone') ? 'iPhone' : navigator.userAgent.includes('iPad') ? 'iPad' : 'Desktop'})`;
  browserNode.connect(nodeWsUrl, nodeName);
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    ws.disconnect();
    browserNode.disconnect();
  });
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
