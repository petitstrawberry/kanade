export * from './stores.svelte';

class BrowserNodeCompat {
  connected = true;

  connect(): void {
    this.connected = true;
  }

  disconnect(): void {
    this.connected = false;
  }

  destroy(): void {
    this.connected = false;
  }
}

export const browserNode = new BrowserNodeCompat();

export function connectBrowserNode(): void {
  browserNode.connect();
}
