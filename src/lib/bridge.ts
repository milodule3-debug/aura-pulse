// Bridge to the Rust backend. Falls back to a mock data generator when
// running in a plain browser (design/dev mode without Tauri).

export const isTauri = "__TAURI_INTERNALS__" in window;

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Snapshot {
  ts: number;
  cpu: { total: number; cores: number[]; freq_mhz: number; temp: number };
  mem: { total: number; used: number; avail: number; swap_total: number; swap_used: number };
  gpu: { present: boolean; busy: number; vram_used: number; vram_total: number; temp: number; watts: number };
  power: { cpu_watts: number; batt_watts: number; batt_pct: number; on_ac: boolean; profile: string };
  disk: { mounts: { path: string; total: number; used: number }[]; read_bps: number; write_bps: number };
  net: { rx_bps: number; tx_bps: number };
  sys: { load1: number; load5: number; uptime: number; procs: number };
  top: { pid: number; name: string; cpu: number; mem: number }[];
}

export interface ClipRow {
  id: number;
  kind: string;
  content: string | null;
  thumb: string | null;
  width: number;
  height: number;
  pinned: boolean;
  created_at: number;
  title: string | null;
  summary: string | null;
  tags: string | null;
  has_ai: boolean;
}

export interface ClipFull extends ClipRow {
  image: string | null;
  ocr: string | null;
  description: string | null;
  markdown: string | null;
  design_json: string | null;
}

// ---------------- mock mode ----------------

let mockClips: ClipFull[] = [];

function mockSnapshot(t: number): Snapshot {
  const wave = (f: number, ph = 0) => (Math.sin(t * f + ph) + 1) / 2;
  const cores = Array.from({ length: 16 }, (_, i) => 100 * Math.max(0, Math.min(1, wave(0.9 + i * 0.11, i) * 0.8 + (Math.random() - 0.5) * 0.2)));
  const total = cores.reduce((a, b) => a + b, 0) / cores.length;
  return {
    ts: Date.now(),
    cpu: { total, cores, freq_mhz: 1800 + wave(0.5) * 2400, temp: 48 + wave(0.3) * 25 },
    mem: { total: 12e9, used: 6e9 + wave(0.2) * 3e9, avail: 4e9, swap_total: 12e9, swap_used: 9e9 + wave(0.1) * 1e9 },
    gpu: { present: true, busy: 100 * wave(0.7, 2) * 0.7, vram_used: 0.7e9 + wave(0.4) * 0.9e9, vram_total: 2.1e9, temp: 44 + wave(0.35, 1) * 22, watts: 4 + wave(0.6) * 12 },
    power: { cpu_watts: 8 + wave(0.8) * 22, batt_watts: 12 + wave(0.4) * 18, batt_pct: 76, on_ac: true, profile: "performance" },
    disk: {
      mounts: [
        { path: "/", total: 155e9, used: 98e9 },
        { path: "/mnt/bigdata", total: 540e9, used: 134e9 },
      ],
      read_bps: wave(1.2) * 90e6,
      write_bps: wave(0.9, 2) * 45e6,
    },
    net: { rx_bps: wave(1.1, 1) * 12e6, tx_bps: wave(0.7, 3) * 2e6 },
    sys: { load1: 2.4 + wave(0.2) * 3, load5: 3.1, uptime: 86400 * 2 + 3600 * 5, procs: 412 },
    top: [
      { pid: 4211, name: "firefox", cpu: 42 * wave(0.5), mem: 1.9e9 },
      { pid: 992, name: "plasmashell", cpu: 18 * wave(0.7, 1), mem: 0.9e9 },
      { pid: 15023, name: "aura-pulse", cpu: 6 * wave(1, 2), mem: 0.2e9 },
      { pid: 2210, name: "kwin_wayland", cpu: 8 * wave(0.9), mem: 0.5e9 },
      { pid: 7305, name: "node", cpu: 12 * wave(0.4, 4), mem: 0.6e9 },
    ],
  };
}

function seedMockClips() {
  const now = Date.now();
  const mk = (id: number, kind: string, content: string, extra: Partial<ClipFull> = {}): ClipFull => ({
    id, kind, content, thumb: null, image: null, width: 0, height: 0, pinned: false,
    created_at: now - id * 7.3e5, title: null, summary: null, tags: null, has_ai: false,
    ocr: null, description: null, markdown: null, design_json: null, ...extra,
  });
  mockClips = [
    mk(1, "code", "fn spawn(app: AppHandle) {\n    std::thread::Builder::new()\n        .name(\"telemetry\".into())\n        .spawn(move || run(app))\n}", { title: "Rust thread spawn", tags: "rust,threading", has_ai: true, summary: "Spawns the telemetry sampling thread." }),
    mk(2, "url", "https://tauri.app/v2/guides/features/events/"),
    mk(3, "text", "Remember: the sphere breathing should sync with CPU load — calm when idle, frantic under stress.", { pinned: true }),
    mk(4, "json", '{"palette":[{"name":"cyan","hex":"#00e5ff"},{"name":"magenta","hex":"#ff2d78"}],"radius":10}'),
    mk(5, "color", "#00e5ff"),
    mk(6, "email", "leanproiq@gmail.com"),
    mk(7, "csv", "model,tokens_s,ram_gb\n7B-q4,11.2,4.4\n3B-q4,24.6,2.0\n1B-q4,65.4,0.8"),
    mk(8, "text", "wl-paste --type text --watch to monitor Wayland clipboard from CLI"),
  ];
}

const mockAi: Record<string, string> = {
  summarize: '{"title":"Mock summary","summary":"This is a simulated AI response (browser mode)","tags":"mock,demo"}',
  ocr: "(mock) EXTRACTED TEXT LINE 1\nEXTRACTED TEXT LINE 2",
  describe: "(mock) A dark interface with neon cyan traces over a deep blue field; rectangular glass panels, sharp corners, a breathing sphere of dots.",
  markdown: "# Mock document\n\nConverted content would appear here.",
  design: '{"palette":[{"name":"cyan","hex":"#00e5ff"}],"typography":{"display":"Orbitron"},"spacing":[4,8,12],"components":[]}',
};

async function mockInvoke(cmd: string, args: any): Promise<any> {
  if (!mockClips.length) seedMockClips();
  await new Promise((r) => setTimeout(r, cmd.startsWith("ai_") || cmd === "bench_run" ? 700 : 60));
  switch (cmd) {
    case "app_os": return "linux";
    case "telemetry_snapshot": return mockSnapshot(performance.now() / 1000);
    case "vault_list": {
      const a = args?.args ?? {};
      let rows = [...mockClips];
      if (a.query) rows = rows.filter((c) => (c.content ?? "").toLowerCase().includes(a.query.toLowerCase()));
      if (a.kind && a.kind !== "all") rows = rows.filter((c) => c.kind === a.kind);
      if (a.pinned_only) rows = rows.filter((c) => c.pinned);
      return rows.sort((x, y) => Number(y.pinned) - Number(x.pinned) || y.created_at - x.created_at);
    }
    case "vault_get": return mockClips.find((c) => c.id === args.id);
    case "vault_delete": mockClips = mockClips.filter((c) => c.id !== args.id); return null;
    case "vault_pin": { const c = mockClips.find((c) => c.id === args.id); if (c) c.pinned = args.pinned; return null; }
    case "vault_wipe": mockClips = []; return null;
    case "vault_copy": return null;
    case "vault_save_as": return "/tmp/vault_clip_" + args.id + ".txt";
    case "vault_add_text": {
      mockClips.unshift({ id: Date.now(), kind: "text", content: args.content, thumb: null, image: null, width: 0, height: 0, pinned: false, created_at: Date.now(), title: null, summary: null, tags: null, has_ai: false, ocr: null, description: null, markdown: null, design_json: null });
      return null;
    }
    case "vault_add_image": {
      mockClips.unshift({ id: Date.now(), kind: "image", content: null, thumb: null, image: args.dataB64 ?? null, width: 64, height: 64, pinned: false, created_at: Date.now(), title: null, summary: null, tags: null, has_ai: false, ocr: null, description: null, markdown: null, design_json: null });
      return mockClips[0].id;
    }
    case "vault_add_audio": {
      mockClips.unshift({ id: Date.now(), kind: "audio", content: `[audio] ${args.name} (mock)`, thumb: null, image: null, width: 0, height: 0, pinned: false, created_at: Date.now(), title: null, summary: null, tags: null, has_ai: false, ocr: null, description: null, markdown: null, design_json: null });
      return mockClips[0].id;
    }
    case "vault_add_path": {
      const isImg = args.kind === "image";
      mockClips.unshift({ id: Date.now(), kind: isImg ? "image" : "audio", content: isImg ? null : `[audio] ${args.path}`, thumb: null, image: null, width: 0, height: 0, pinned: false, created_at: Date.now(), title: null, summary: null, tags: null, has_ai: false, ocr: null, description: null, markdown: null, design_json: null });
      return mockClips[0].id;
    }
    case "ai_transcribe": {
      const c = mockClips.find((c) => c.id === args.clipId);
      const describe = args.mode === "describe";
      const text = describe
        ? "(mock) Upbeat synth track, female vocal, driving beat — simulated description."
        : "(mock) This is a simulated transcript of the audio clip.";
      if (c) {
        if (describe) c.description = text;
        else c.ocr = text;
        c.has_ai = true;
      }
      return text;
    }
    case "vault_stats": return { total: mockClips.length, pinned: mockClips.filter((c) => c.pinned).length, by_kind: Object.entries(mockClips.reduce((m: any, c) => ((m[c.kind] = (m[c.kind] ?? 0) + 1), m), {})), db_bytes: 482304 };
    case "ai_get_config": return { active: "ollama", providers: { openai: { kind: "openai", label: "OpenAI", api_key: "", model: "gpt-4o-mini", base_url: "https://api.openai.com/v1" }, anthropic: { kind: "anthropic", label: "Anthropic", api_key: "", model: "claude-sonnet-5", base_url: "https://api.anthropic.com" }, ollama: { kind: "openai", label: "Ollama (local)", api_key: "", model: "llama3.2", base_url: "http://127.0.0.1:11434/v1" }, zhipu: { kind: "openai", label: "ZhiPu GLM-5.2", api_key: "", model: "glm-5.2", base_url: "https://open.bigmodel.cn/api/coding/paas/v4" } } };
    case "ai_set_config": return null;
    case "ai_test": return { ok: true, latency_ms: 420, message: "AURA LINK OK (mock)" };
    case "ai_run": { const c = mockClips.find((c) => c.id === args.clipId); if (c) c.has_ai = true; return mockAi[args.task] ?? "(mock reply)"; }
    case "ai_chat": return "(mock) Aura here — this is a simulated reply. Run inside Tauri for real AI calls.";
    case "bench_run":
      if (args.test === "cpu") return { single_mops: 412, multi_mops: 4620, threads: 16, scaling: 11.2 };
      if (args.test === "memory") return { gbps_read: 21.4, gbps_write: 14.2, gbps_copy: 17.8 };
      if (args.test === "disk") return { write_mbps: 1840, read_mbps: 3900, note: "read is OS-cache assisted", path: "~/.local/share/aura-pulse" };
      return { bandwidth_gbps: 17.8, effective_gbps: 12.8, estimates: [{ model: "1B q4", tok_s: 17, fits_ram: true }, { model: "3B q4", tok_s: 6.4, fits_ram: true }, { model: "7B q4", tok_s: 2.9, fits_ram: true }, { model: "13B q4", tok_s: 1.6, fits_ram: true }, { model: "34B q4", tok_s: 0.65, fits_ram: false }], ollama: { model: "llama3.2:latest", tok_s: 9.8, prompt_tok_s: 61 }, lmstudio: { model: "local-model", tok_s: 12.4, prompt_tok_s: 0 } };
    case "ai_optimize_generate":
      return {
        state: "loadavg: 2.41 3.02 3.11\ncpu: 16 threads, Mock CPU\ncpu governor: performance\nMemTotal: 12000000 kB\nMemAvailable: 3200000 kB\nSwapTotal: 12000000 kB\nSwapFree: 2600000 kB\nvm.swappiness: 60\ndisk /: 155 GB total, 41 GB free",
        filtered: 1,
        modules: [
          { title: "Lower swappiness for desktop use", rationale: "9.4 GB of swap is in use with swappiness 60 — lowering to 10 keeps active apps in RAM.", impact: "high", risk: "safe", requires_root: true, commands: ["sysctl -w vm.swappiness=10"] },
          { title: "Enable scheduler autogroup", rationale: "Improves desktop responsiveness under the current load of 2.4 by grouping per-session tasks.", impact: "medium", risk: "safe", requires_root: true, commands: ["sysctl -w kernel.sched_autogroup_enabled=1"] },
        ],
      };
    case "ai_optimize_apply": return "(mock) vm.swappiness = 10";
    case "sysopt_get": return { profile: "performance", profiles: ["performance", "balanced", "power-saver"], governor: "performance", epp: "performance", boost: true, swappiness: 60, has_ppd: true };
    case "sysopt_balance_cores": return "cores onlined: 0 | governor: performance (all cores) | autogroup: on | IRQ spread: rebalanced";
    case "sysopt_set_profile": case "sysopt_set_boost": case "sysopt_set_swappiness": case "sysopt_drop_caches": return null;
    default: throw new Error(`mock: unhandled command ${cmd}`);
  }
}

// ---------------- public API ----------------

// Live activity signals consumed by the Global Sphere and status caption.
export const activity = {
  ai: 0, // in-flight model calls
  vaultAt: 0, // performance.now() of the last clipboard capture
};

const AI_MODEL_CMDS = new Set(["ai_test", "ai_run", "ai_chat", "ai_transcribe", "ai_optimize_generate"]);

export async function call<T = any>(cmd: string, args?: Record<string, any>): Promise<T> {
  const tracked = AI_MODEL_CMDS.has(cmd);
  if (tracked) activity.ai++;
  try {
    if (isTauri) return await tauriInvoke<T>(cmd, args);
    return (await mockInvoke(cmd, args)) as T;
  } finally {
    if (tracked) activity.ai--;
  }
}

type Unsub = () => void;

// Native (Tauri) file drag-drop. With dragDropEnabled (the default) the
// webview swallows HTML5 drop events, so file drops only arrive through
// this event — as filesystem paths, with positions in physical pixels.
export type DragDropInfo =
  | { type: "over"; x: number; y: number }
  | { type: "drop"; paths: string[]; x: number; y: number }
  | { type: "leave" };

export function onDragDrop(cb: (e: DragDropInfo) => void): Unsub {
  if (!isTauri) return () => {};
  let un: (() => void) | null = null;
  let dead = false;
  import("@tauri-apps/api/webview").then(async ({ getCurrentWebview }) => {
    const u = await getCurrentWebview().onDragDropEvent((ev) => {
      const p = ev.payload;
      const scale = window.devicePixelRatio || 1;
      if (p.type === "enter" || p.type === "over") {
        cb({ type: "over", x: p.position.x / scale, y: p.position.y / scale });
      } else if (p.type === "drop") {
        cb({ type: "drop", paths: p.paths, x: p.position.x / scale, y: p.position.y / scale });
      } else {
        cb({ type: "leave" });
      }
    });
    if (dead) u();
    else un = u;
  });
  return () => {
    dead = true;
    un?.();
  };
}

export function onTelemetry(cb: (s: Snapshot) => void): Unsub {
  if (isTauri) {
    let un: (() => void) | null = null;
    listen<Snapshot>("telemetry", (e) => cb(e.payload)).then((u) => (un = u));
    return () => un?.();
  }
  const id = setInterval(() => cb(mockSnapshot(performance.now() / 1000)), 100);
  return () => clearInterval(id);
}

export function onEvent<T = any>(name: string, cb: (payload: T) => void): Unsub {
  if (isTauri) {
    let un: (() => void) | null = null;
    listen<T>(name, (e) => cb(e.payload)).then((u) => (un = u));
    return () => un?.();
  }
  return () => {};
}

// Shared latest-snapshot store so components can read without subscribing.
export const latest: { snap: Snapshot | null } = { snap: null };
onTelemetry((s) => (latest.snap = s));
onEvent("vault_changed", () => (activity.vaultAt = performance.now()));
