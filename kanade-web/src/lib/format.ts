export function formatDuration(seconds: number | null): string {
  if (seconds == null || isNaN(seconds)) return "0:00";
  const s = Math.floor(seconds);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${r.toString().padStart(2, "0")}`;
}

export function formatSampleRate(rate: number | null): string {
  if (!rate) return "";
  return `${(rate / 1000).toFixed(1)} kHz`;
}
