import { WsClient } from './ws';

const params = new URLSearchParams(window.location.search);
const wsUrl = params.get('server') || 'ws://127.0.0.1:8080';
export const mediaBase = params.get('media') || 'http://127.0.0.1:8081';

export const ws = new WsClient(wsUrl);

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
