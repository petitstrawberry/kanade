const MEDIA_SESSION_COOKIE_NAME = 'kanade_session';

function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) {
    throw new Error('Invalid hex string length');
  }

  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    const byte = Number.parseInt(hex.slice(i, i + 2), 16);
    if (Number.isNaN(byte)) {
      throw new Error('Invalid hex string');
    }
    bytes[i / 2] = byte;
  }
  return bytes;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

function withTrailingSlash(url: string): string {
  return url.endsWith('/') ? url : `${url}/`;
}

export function buildMediaUrl(mediaBase: string, path: string): string {
  return new URL(path.replace(/^\//, ''), withTrailingSlash(mediaBase)).toString();
}

export function mediaBaseUsesCurrentHost(mediaBase: string): boolean {
  return new URL(mediaBase).hostname === window.location.hostname;
}

export async function computeMediaSessionCookieValue(mediaAuthKeyHex: string, mediaAuthKeyId: string): Promise<string> {
  const keyBytes = hexToBytes(mediaAuthKeyHex);
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    keyBytes,
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const signature = await crypto.subtle.sign('HMAC', cryptoKey, new TextEncoder().encode(mediaAuthKeyId));
  return `${mediaAuthKeyId}:${bytesToHex(new Uint8Array(signature))}`;
}

export function setMediaSessionCookie(cookieValue: string, mediaBase: string): void {
  const mediaHost = new URL(mediaBase).hostname;
  const isSecure = new URL(mediaBase).protocol === 'https:';
  const sameSite = mediaHost === window.location.hostname ? 'Lax' : 'None';
  document.cookie = `${MEDIA_SESSION_COOKIE_NAME}=${cookieValue}; path=/; SameSite=${sameSite}${isSecure ? '; Secure' : ''}`;
}

export function clearMediaSessionCookie(mediaBase?: string): void {
  const sameSite = mediaBase && new URL(mediaBase).hostname !== window.location.hostname ? 'None' : 'Lax';
  const isSecure = mediaBase && new URL(mediaBase).protocol === 'https:';
  document.cookie = `${MEDIA_SESSION_COOKIE_NAME}=; path=/; Max-Age=0; expires=Thu, 01 Jan 1970 00:00:00; SameSite=${sameSite}${isSecure ? '; Secure' : ''}`;
}
