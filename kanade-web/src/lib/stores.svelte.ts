import { WsClient } from './ws';
import { AudioPlayer } from './audio-player';
import { BrowserNode } from './browser-node';

const params = new URLSearchParams(window.location.search);
const wsScheme = location.protocol === 'https:' ? 'wss' : 'ws';
const httpScheme = location.protocol === 'https:' ? 'https' : 'http';
const host = location.host;
const SERVER_STORAGE_KEY = 'kanade_server';
const sameOriginWsFallback = `${wsScheme}://${host}/ws`;
const sameOriginMediaFallback = `${httpScheme}://${host}`;

function normalizeSetting(raw: string | null | undefined): string | null {
  const value = raw?.trim();
  return value ? value : null;
}

function normalizeUrl(raw: string | null, fallback: string, scheme: string): string {
  const value = normalizeSetting(raw);
  if (!value) return fallback;
  if (/^[a-zA-Z][a-zA-Z\d+.-]*:\/\//.test(value)) return value;
  return `${scheme}://${value}`;
}

function buildWsUrl(raw: string | null, fallback: string): string {
  const base = normalizeUrl(raw, fallback, wsScheme);
  if (base.endsWith('/ws')) return base;
  return `${base.replace(/\/+$/, '')}/ws`;
}

function getServerQueryValue(): string | null {
  return normalizeSetting(params.get('server'));
}

function getStoredValue(key: string): string | null {
  try {
    return normalizeSetting(window.localStorage.getItem(key));
  } catch {
    return null;
  }
}

function setStoredValue(key: string, value: string): void {
  const normalized = normalizeSetting(value);
  try {
    if (normalized) {
      window.localStorage.setItem(key, normalized);
    } else {
      window.localStorage.removeItem(key);
    }
  } catch {}
}

function resolveServerValue(saved: string | null): string | null {
  return getServerQueryValue() ?? saved;
}

const initialSavedServer = getStoredValue(SERVER_STORAGE_KEY);

let currentWsUrl = $state(buildWsUrl(resolveServerValue(initialSavedServer), sameOriginWsFallback));
let mediaBase = $state(sameOriginMediaFallback);

export const ws = new WsClient(currentWsUrl, mediaBase);

const player = new AudioPlayer(() => {});
export function getPlayer(): AudioPlayer {
  return player;
}

export class ConnectionSettings {
  open = $state(false);
  serverInput = $state(getServerQueryValue() ?? initialSavedServer ?? host);
  savedServer = $state(initialSavedServer ?? '');

  get serverQueryOverride(): string | null {
    return getServerQueryValue();
  }

  get hasServerQueryOverride(): boolean {
    return this.serverQueryOverride !== null;
  }

  get effectiveServerValue(): string {
    return this.serverQueryOverride ?? normalizeSetting(this.savedServer) ?? host;
  }

  get wsUrl(): string {
    return currentWsUrl;
  }

  openPanel(): void {
    this.open = true;
  }

  closePanel(): void {
    this.open = false;
  }

  save(): void {
    const nextSavedServer = normalizeSetting(this.serverInput);

    this.savedServer = nextSavedServer ?? '';
    setStoredValue(SERVER_STORAGE_KEY, this.savedServer);

    reconnectClients(nextSavedServer);
    showToast(this.hasServerQueryOverride
      ? 'Settings saved. Query params still override this session.'
      : 'Connection settings saved.');
  }

  clear(): void {
    this.savedServer = '';
    this.serverInput = this.serverQueryOverride ?? host;
    setStoredValue(SERVER_STORAGE_KEY, '');

    reconnectClients(null);
    showToast('Saved connection settings cleared.');
  }

  disconnect(): void {
    ws.disconnect();
    browserNode.disconnect();
    showToast('Disconnected.');
  }
}

export const connectionSettings = new ConnectionSettings();

export const browserNode = new BrowserNode(
  player,
  () => ws.mediaRequestsReady,
  (listener) => ws.onMediaAuthChange(listener),
);

export function getMediaBase(): string {
  return mediaBase;
}

export function updateMediaBase(newBase: string): string {
  mediaBase = normalizeUrl(newBase, sameOriginMediaFallback, httpScheme);
  ws.updateMediaBase(mediaBase);
  return mediaBase;
}

function reconnectClients(savedServer: string | null): void {
  currentWsUrl = buildWsUrl(resolveServerValue(savedServer), sameOriginWsFallback);
  mediaBase = sameOriginMediaFallback;
  ws.reconnectTo(currentWsUrl, mediaBase);
  connectBrowserNode();
}

export function connectBrowserNode(): void {
  const nodeWs = new URL(currentWsUrl);
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
    browserNode.destroy();
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
