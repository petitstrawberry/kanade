function withTrailingSlash(url: string): string {
  return url.endsWith('/') ? url : `${url}/`;
}

export function buildMediaUrl(mediaBase: string, path: string): string {
  return new URL(path.replace(/^\//, ''), withTrailingSlash(mediaBase)).toString();
}
