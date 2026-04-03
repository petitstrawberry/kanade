import { WsClient } from './ws';

const params = new URLSearchParams(window.location.search);
const wsUrl = params.get('server') || 'ws://127.0.0.1:8080';
export const mediaBase = params.get('media') || 'http://127.0.0.1:8081';
export const nodeId = 'default';

export const ws = new WsClient(wsUrl);

export class ActiveTab {
  value = $state<'now-playing' | 'library' | 'queue' | 'search'>('now-playing');
}
