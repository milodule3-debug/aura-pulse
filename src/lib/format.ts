export function fmtBytes(n: number, digits = 1): string {
  if (!isFinite(n) || n < 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(n >= 100 || i === 0 ? 0 : digits)} ${units[i]}`;
}

export function fmtRate(bps: number): string {
  if (!isFinite(bps) || bps < 0) return "—";
  if (bps < 1e3) return `${bps.toFixed(0)} B/s`;
  if (bps < 1e6) return `${(bps / 1e3).toFixed(1)} KB/s`;
  if (bps < 1e9) return `${(bps / 1e6).toFixed(1)} MB/s`;
  return `${(bps / 1e9).toFixed(2)} GB/s`;
}

export function fmtUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return d > 0 ? `${d}d ${h}h ${m}m` : h > 0 ? `${h}h ${m}m` : `${m}m`;
}

export function fmtAgo(ts: number): string {
  const s = Math.max(0, (Date.now() - ts) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  const d = new Date(ts);
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" }) +
    " " + d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function fmtNum(n: number, digits = 1): string {
  if (!isFinite(n)) return "—";
  return n >= 1000 ? n.toLocaleString(undefined, { maximumFractionDigits: 0 }) : n.toFixed(digits);
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}
