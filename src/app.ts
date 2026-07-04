// Application shell: topbar with tabs, retractable sidebar hosting the
// Global Sphere and quick controls, and the routed main view area.

import { activity, call, onTelemetry, Snapshot } from "./lib/bridge";
import { fmtBytes, fmtUptime } from "./lib/format";
import { h, icon, toast } from "./lib/ui";
import { GlobalSphere } from "./lib/sphere";
import { DiagnosticsView } from "./views/diagnostics";
import { VaultView } from "./views/vault";
import { BenchView } from "./views/bench";
import { OptimizeView } from "./views/optimize";
import { SettingsView } from "./views/settings";

export interface View {
  mount(root: HTMLElement): void;
  unmount(): void;
}

const TABS = [
  { id: "diagnostics", label: "Diagnostics", icon: "gauge" },
  { id: "bench", label: "Benchmarks", icon: "zap" },
  { id: "optimize", label: "Optimization", icon: "chip" },
  { id: "vault", label: "The Vault", icon: "vault" },
];

export class App {
  private root: HTMLElement;
  private main!: HTMLElement;
  private sidebar!: HTMLElement;
  private tabsEl: Map<string, HTMLElement> = new Map();
  private current: View | null = null;
  private currentId = "";
  private views: Record<string, () => View> = {
    diagnostics: () => new DiagnosticsView(),
    vault: () => new VaultView(),
    bench: () => new BenchView(),
    optimize: () => new OptimizeView(),
    settings: () => new SettingsView(),
  };
  private sphere: GlobalSphere | null = null;
  private unsubs: (() => void)[] = [];

  constructor(root: HTMLElement) {
    this.root = root;
    this.build();
    const initial = location.hash.slice(1);
    this.navigate(this.views[initial] ? initial : "diagnostics");
  }

  private build() {
    // ---- topbar ----
    const cpuStat = h("div", { class: "top-stat" }, h("span", { class: "k" }, "CPU"), h("span", { class: "v" }, "—"));
    const memStat = h("div", { class: "top-stat" }, h("span", { class: "k" }, "MEM"), h("span", { class: "v" }, "—"));
    const pwrStat = h("div", { class: "top-stat" }, h("span", { class: "k" }, "PWR"), h("span", { class: "v" }, "—"));
    const clock = h("div", { class: "clock" }, "--:--:--");

    const tabsWrap = h("div", { class: "tabs" });
    for (const t of TABS) {
      const el = h("button", { class: "tab", onclick: () => this.navigate(t.id) }, icon(t.icon), t.label);
      this.tabsEl.set(t.id, el);
      tabsWrap.append(el);
    }

    const sidebarBtn = h(
      "button",
      { class: "icon-btn", title: "Toggle sidebar", onclick: () => this.sidebar.classList.toggle("hidden") },
      icon("panel"),
    );
    const settingsBtn = h(
      "button",
      { class: "icon-btn", title: "AI Configuration Hub", onclick: () => this.navigate("settings") },
      icon("settings"),
    );

    const topbar = h(
      "header",
      { class: "topbar" },
      sidebarBtn,
      h(
        "div",
        { class: "brand", onclick: () => this.navigate("diagnostics") },
        (() => {
          const i = icon("pulse", 26);
          i.style.color = "var(--cyan)";
          return i;
        })(),
        h("div", { class: "name", html: "AURA&nbsp;<b>PULSE</b>" }),
      ),
      tabsWrap,
      h("div", { class: "spacer" }),
      cpuStat,
      memStat,
      pwrStat,
      settingsBtn,
      clock,
    );

    // ---- sidebar ----
    const sphereCanvas = h("canvas");
    const sphereCaption = h("div", { class: "sphere-caption" }, "Global Sphere");
    this.sidebar = h(
      "aside",
      { class: "sidebar" },
      h("div", { class: "sphere-wrap" }, sphereCanvas, sphereCaption),
      this.buildSideSystem(),
      this.buildSidePower(),
      this.buildSideAi(),
    );

    this.main = h("main", { class: "main" });
    this.root.append(topbar, h("div", { class: "body" }, this.sidebar, this.main));

    this.sphere = new GlobalSphere(sphereCanvas as HTMLCanvasElement);

    // Platform adaptation: the Optimization tab is Linux-only (pkexec,
    // sysfs, power-profiles-daemon have no Windows counterparts yet).
    call<string>("app_os")
      .then((os) => {
        if (os === "windows") {
          this.tabsEl.get("optimize")?.remove();
          this.tabsEl.delete("optimize");
          delete this.views["optimize"];
          if (this.currentId === "optimize") this.navigate("diagnostics");
        }
      })
      .catch(() => {});

    // live topbar stats
    this.unsubs.push(
      onTelemetry((s: Snapshot) => {
        (cpuStat.querySelector(".v") as HTMLElement).textContent = `${s.cpu.total.toFixed(0)}%`;
        (memStat.querySelector(".v") as HTMLElement).textContent = `${((s.mem.used / s.mem.total) * 100).toFixed(0)}%`;
        (pwrStat.querySelector(".v") as HTMLElement).textContent = `${(s.power.cpu_watts + s.gpu.watts).toFixed(1)}W`;
      }),
    );
    const tickClock = () => (clock.textContent = new Date().toLocaleTimeString(undefined, { hour12: false }));
    tickClock();
    setInterval(tickClock, 1000);

    // sphere caption reflects live activity
    const tickCaption = () => {
      const vaultFresh = activity.vaultAt > 0 && performance.now() - activity.vaultAt < 3000;
      sphereCaption.textContent = activity.ai > 0 ? "AI Link Active" : vaultFresh ? "Vault Capture" : "Global Sphere";
      sphereCaption.classList.toggle("hot", activity.ai > 0);
      sphereCaption.classList.toggle("warm", activity.ai === 0 && vaultFresh);
    };
    setInterval(tickCaption, 500);
  }

  private buildSideSystem(): HTMLElement {
    const uptime = h("span", { class: "v" }, "—");
    const load = h("span", { class: "v" }, "—");
    const procs = h("span", { class: "v" }, "—");
    const batt = h("span", { class: "v" }, "—");
    const swap = h("span", { class: "v" }, "—");
    this.unsubs.push(
      onTelemetry((s) => {
        uptime.textContent = fmtUptime(s.sys.uptime);
        load.textContent = `${s.sys.load1.toFixed(2)} / ${s.sys.load5.toFixed(2)}`;
        procs.textContent = String(s.sys.procs);
        batt.textContent = `${s.power.batt_pct.toFixed(0)}%${s.power.on_ac ? " ⚡" : ""}`;
        swap.textContent = fmtBytes(s.mem.swap_used);
      }),
    );
    const kv = (k: string, v: HTMLElement) => h("div", { class: "kv" }, h("span", { class: "k" }, k), v);
    return h(
      "div",
      { class: "side-section" },
      h("div", { class: "side-title" }, "System"),
      kv("Uptime", uptime),
      kv("Load 1m / 5m", load),
      kv("Processes", procs),
      kv("Battery", batt),
      kv("Swap used", swap),
    );
  }

  private buildSidePower(): HTMLElement {
    const seg = h("div", { class: "seg" });
    const profiles = ["power-saver", "balanced", "performance"];
    const labels: Record<string, string> = { "power-saver": "Saver", balanced: "Balance", performance: "Perf" };
    const btns = new Map<string, HTMLElement>();
    for (const p of profiles) {
      const b = h(
        "button",
        {
          onclick: async () => {
            try {
              await call("sysopt_set_profile", { profile: p });
              toast(`Power profile → ${p}`);
            } catch (e) {
              toast(String(e), true);
            }
          },
        },
        labels[p],
      );
      btns.set(p, b);
      seg.append(b);
    }
    this.unsubs.push(
      onTelemetry((s) => {
        for (const [p, b] of btns) b.classList.toggle("active", s.power.profile === p);
      }),
    );
    return h("div", { class: "side-section" }, h("div", { class: "side-title" }, "Power Profile"), seg);
  }

  private buildSideAi(): HTMLElement {
    const sel = h("select", { style: { width: "100%" } }) as HTMLSelectElement;
    sel.onchange = async () => {
      try {
        const cfg = await call<any>("ai_get_config");
        cfg.active = sel.value;
        await call("ai_set_config", { cfg });
        toast(`AI core → ${sel.value}`);
      } catch (e) {
        toast(String(e), true);
      }
    };
    const refresh = async () => {
      try {
        const cfg = await call<any>("ai_get_config");
        sel.innerHTML = "";
        for (const [key, p] of Object.entries<any>(cfg.providers)) {
          sel.append(h("option", { value: key, ...(cfg.active === key ? { selected: "" } : {}) }, p.label));
        }
      } catch { /* backend not ready */ }
    };
    refresh();
    window.addEventListener("ai-config-changed", refresh);

    return h(
      "div",
      { class: "side-section" },
      h("div", { class: "side-title" }, "AI Core"),
      sel,
      h("div", { style: { height: "10px" } }),
      h(
        "button",
        { class: "btn ghost", style: { width: "100%", justifyContent: "center" }, onclick: () => this.navigate("settings") },
        icon("settings"),
        "Configuration Hub",
      ),
    );
  }

  // Views are mounted once and kept alive (hidden) across tab switches so
  // state survives: bench results, generated modules, chart history.
  private mounted = new Map<string, { view: View; wrap: HTMLElement }>();

  navigate(id: string) {
    if (this.currentId === id) return;
    if (!this.views[id]) return;
    this.currentId = id;
    location.hash = id;
    for (const [tid, el] of this.tabsEl) el.classList.toggle("active", tid === id);
    for (const [, m] of this.mounted) m.wrap.style.display = "none";
    let m = this.mounted.get(id);
    if (!m) {
      const view = this.views[id]();
      const wrap = h("div", { class: "view" });
      this.main.append(wrap);
      view.mount(wrap);
      m = { view, wrap };
      this.mounted.set(id, m);
    }
    m.wrap.style.display = "";
    this.current = m.view;
  }
}
