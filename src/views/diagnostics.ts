// Diagnostics: fluid telemetry dashboard fed at 10 Hz.

import { onTelemetry, Snapshot } from "../lib/bridge";
import { RingGauge, StripChart } from "../lib/charts";
import { fmtBytes, fmtRate } from "../lib/format";
import { h, icon } from "../lib/ui";
import type { View } from "../app";

function panel(iconName: string, title: string, live: HTMLElement | null, body: HTMLElement, span: string): HTMLElement {
  return h(
    "section",
    { class: `panel ${span}` },
    h(
      "div",
      { class: "panel-head" },
      icon(iconName),
      h("span", { class: "t" }, title),
      h("div", { class: "spacer" }),
      live ?? "",
    ),
    h("div", { class: "panel-body" }, body),
  );
}

function readout(label: string, cls = ""): { el: HTMLElement; v: HTMLElement } {
  const v = h("span", { class: "v" }, "—");
  return { el: h("div", { class: `readout ${cls}` }, h("span", { class: "k" }, label), v), v };
}

export class DiagnosticsView implements View {
  private unsub: (() => void) | null = null;
  private charts: StripChart[] = [];
  private freqDiv = Number(localStorage.getItem("ap-telem-div")) || 1;
  private freqHandler = ((e: Event) => { this.freqDiv = (e as CustomEvent).detail; }) as EventListener;

  mount(root: HTMLElement) {
    window.addEventListener("ap-freq-changed", this.freqHandler);
    // ---------- CPU ----------
    const cpuLive = h("span", { class: "live" }, "—");
    const cpuCanvas = h("canvas", { class: "chart-canvas" }) as HTMLCanvasElement;
    const cpuChart = new StripChart(cpuCanvas, {
      series: [{ color: "#00e5ff", fill: true }],
      height: 130,
    });
    const rFreq = readout("Frequency", "accent");
    const rTemp = readout("Temp", "amber");
    const rLoad = readout("Load 1m");
    const coreGrid = h("div", { class: "cores-grid" });
    const coreCells: { bar: HTMLElement; label: HTMLElement }[] = [];
    const cpuBody = h(
      "div",
      {},
      h("div", { class: "readout-row" }, rFreq.el, rTemp.el, rLoad.el),
      cpuCanvas,
      coreGrid,
    );

    // ---------- memory ----------
    const memLive = h("span", { class: "live" }, "—");
    const memCanvas = h("canvas", { class: "chart-canvas" }) as HTMLCanvasElement;
    const memChart = new StripChart(memCanvas, {
      series: [
        { color: "#00e5ff", fill: true },
        { color: "#ff2d78", fill: true },
      ],
      height: 130,
    });
    const rMem = readout("RAM used", "accent");
    const rSwap = readout("Swap", "mag");
    const rAvail = readout("Available");
    const memBody = h(
      "div",
      {},
      h("div", { class: "readout-row" }, rMem.el, rSwap.el, rAvail.el),
      memCanvas,
    );

    // ---------- GPU ----------
    const gpuLive = h("span", { class: "live" }, "—");
    const gpuCanvas = h("canvas", { class: "chart-canvas" }) as HTMLCanvasElement;
    const gpuChart = new StripChart(gpuCanvas, {
      series: [{ color: "#9d6bff", fill: true }],
      height: 110,
    });
    const rVram = readout("VRAM", "accent");
    const rGpuT = readout("Temp", "amber");
    const rGpuW = readout("Power", "lime");
    const vramMeter = h("div", { class: "prog", style: { marginTop: "8px" } }, h("div"));
    const gpuBody = h(
      "div",
      {},
      h("div", { class: "readout-row" }, rVram.el, rGpuT.el, rGpuW.el),
      gpuCanvas,
      vramMeter,
    );

    // ---------- thermals / gauges ----------
    const gCpuT = h("canvas") as HTMLCanvasElement;
    const gGpuT = h("canvas") as HTMLCanvasElement;
    const gBatt = h("canvas") as HTMLCanvasElement;
    const gaugeCpu = new RingGauge(gCpuT, "CPU temp", "°C", 105, 100);
    const gaugeGpu = new RingGauge(gGpuT, "GPU temp", "°C", 105, 100);
    const gaugeBatt = new RingGauge(gBatt, "Battery", "%", 100, 100);
    const gaugeBody = h("div", { class: "gauge-row" }, gCpuT, gGpuT, gBatt);

    // ---------- power ----------
    const pwrLive = h("span", { class: "live" }, "—");
    const pwrCanvas = h("canvas", { class: "chart-canvas" }) as HTMLCanvasElement;
    const pwrChart = new StripChart(pwrCanvas, {
      series: [
        { color: "#00e5ff", fill: true },
        { color: "#ffb02e" },
      ],
      max: "auto",
      height: 110,
    });
    const rCpuW = readout("CPU pkg", "accent");
    const rBattW = readout("Battery draw", "amber");
    const rProfile = readout("Profile");
    const pwrBody = h(
      "div",
      {},
      h("div", { class: "readout-row" }, rCpuW.el, rBattW.el, rProfile.el),
      pwrCanvas,
    );

    // ---------- disk ----------
    const diskLive = h("span", { class: "live" }, "—");
    const diskCanvas = h("canvas", { class: "chart-canvas" }) as HTMLCanvasElement;
    const diskChart = new StripChart(diskCanvas, {
      series: [{ color: "#47ffa0", fill: true }, { color: "#ff2d78", fill: true }],
      max: "auto",
      height: 96,
    });
    const mountsWrap = h("div", { style: { paddingTop: "6px" } });
    const diskBody = h(
      "div",
      {},
      h(
        "div",
        { class: "readout-row" },
        h("div", { class: "readout lime" }, h("span", { class: "k" }, "Read"), h("span", { class: "v", "data-r": "" }, "—")),
        h("div", { class: "readout mag" }, h("span", { class: "k" }, "Write"), h("span", { class: "v", "data-w": "" }, "—")),
      ),
      diskCanvas,
      mountsWrap,
    );

    // ---------- network ----------
    const netCanvas = h("canvas", { class: "chart-canvas" }) as HTMLCanvasElement;
    const netChart = new StripChart(netCanvas, {
      series: [{ color: "#00e5ff", fill: true }, { color: "#ff2d78", fill: true }],
      max: "auto",
      height: 96,
    });
    const rRx = readout("Down", "accent");
    const rTx = readout("Up", "mag");
    const netBody = h("div", {}, h("div", { class: "readout-row" }, rRx.el, rTx.el), netCanvas);

    // ---------- processes ----------
    const procTbody = h("tbody");
    const procBody = h(
      "div",
      {},
      h(
        "table",
        { class: "proc-table" },
        h(
          "thead",
          {},
          h("tr", {}, h("th", {}, "PID"), h("th", {}, "Process"), h("th", {}, "CPU %"), h("th", {}, "Memory")),
        ),
        procTbody,
      ),
    );

    this.charts = [cpuChart, memChart, gpuChart, pwrChart, diskChart, netChart];

    root.append(
      h(
        "div",
        { class: "view-header" },
        h("h2", { html: "SYSTEM <b>DIAGNOSTICS</b>" }),
        h("span", { class: "sub" }, "0.1s telemetry · live"),
      ),
      h(
        "div",
        { class: "diag-grid" },
        panel("cpu", "CPU Matrix", cpuLive, cpuBody, "span-7"),
        panel("mem", "Memory Banks", memLive, memBody, "span-5"),
        panel("chip", "GPU Core", gpuLive, gpuBody, "span-4"),
        panel("therm", "Thermals", null, gaugeBody, "span-4"),
        panel("zap", "Power Draw", pwrLive, pwrBody, "span-4"),
        panel("disk", "Storage Array", diskLive, diskBody, "span-4"),
        panel("net", "Network Link", null, netBody, "span-4"),
        panel("gauge", "Top Processes", null, procBody, "span-4"),
      ),
    );

    let frame = 0;
    this.unsub = onTelemetry((s: Snapshot) => {
      frame++;
      // throttle rendering to user-chosen frequency
      if (this.freqDiv > 1 && frame % this.freqDiv !== 0) return;

      // CPU
      cpuLive.textContent = `${s.cpu.total.toFixed(1)}%`;
      cpuChart.push([s.cpu.total]);
      rFreq.v.innerHTML = `${(s.cpu.freq_mhz / 1000).toFixed(2)}<small> GHz</small>`;
      rTemp.v.innerHTML = `${s.cpu.temp.toFixed(0)}<small> °C</small>`;
      rLoad.v.textContent = s.sys.load1.toFixed(2);
      if (coreCells.length !== s.cpu.cores.length) {
        coreGrid.innerHTML = "";
        coreCells.length = 0;
        s.cpu.cores.forEach((_, i) => {
          const bar = h("div");
          const label = h("span", {}, String(i));
          coreGrid.append(h("div", { class: "core-cell" }, bar, label));
          coreCells.push({ bar, label });
        });
      }
      s.cpu.cores.forEach((c, i) => {
        coreCells[i].bar.style.height = `${c}%`;
        coreCells[i].bar.style.background =
          c > 85 ? "linear-gradient(180deg,#ff3b5c,#7a1020)" : c > 60 ? "linear-gradient(180deg,#ffb02e,#7a4a00)" : "";
      });

      // memory
      const memPct = (s.mem.used / s.mem.total) * 100;
      const swapPct = s.mem.swap_total > 0 ? (s.mem.swap_used / s.mem.swap_total) * 100 : 0;
      memLive.textContent = `${memPct.toFixed(1)}%`;
      memChart.push([memPct, swapPct]);
      rMem.v.textContent = fmtBytes(s.mem.used);
      rSwap.v.textContent = fmtBytes(s.mem.swap_used);
      rAvail.v.textContent = fmtBytes(s.mem.avail);

      // GPU
      if (s.gpu.present) {
        gpuLive.textContent = `${s.gpu.busy.toFixed(0)}%`;
        gpuChart.push([s.gpu.busy]);
        const vp = s.gpu.vram_total > 0 ? (s.gpu.vram_used / s.gpu.vram_total) * 100 : 0;
        rVram.v.innerHTML = `${fmtBytes(s.gpu.vram_used)}<small> / ${fmtBytes(s.gpu.vram_total)}</small>`;
        rGpuT.v.innerHTML = `${s.gpu.temp.toFixed(0)}<small> °C</small>`;
        rGpuW.v.innerHTML = `${s.gpu.watts.toFixed(1)}<small> W</small>`;
        (vramMeter.firstElementChild as HTMLElement).style.width = `${vp}%`;
      }

      // gauges (2 Hz is plenty)
      if (frame % 5 === 0) {
        gaugeCpu.set(s.cpu.temp);
        gaugeGpu.set(s.gpu.temp);
        gaugeBatt.set(s.power.batt_pct);
      }

      // power — on APUs the amdgpu sensor reports the whole package and
      // RAPL is often root-only; adapt labels to whatever is available
      const apuMode = s.power.cpu_watts < 0.05 && s.gpu.watts > 0;
      const pkgW = apuMode ? s.gpu.watts : s.power.cpu_watts;
      const totW = apuMode ? s.gpu.watts : s.power.cpu_watts + s.gpu.watts;
      pwrLive.textContent = `${totW.toFixed(1)}W`;
      pwrChart.push([pkgW, s.power.batt_watts]);
      (rCpuW.el.querySelector(".k") as HTMLElement).textContent = apuMode ? "APU pkg" : "CPU pkg";
      rCpuW.v.innerHTML = `${pkgW.toFixed(1)}<small> W</small>`;
      rBattW.v.innerHTML = `${s.power.batt_watts.toFixed(1)}<small> W</small>`;
      rProfile.v.textContent = s.power.profile || "—";

      // disk
      diskLive.textContent = fmtRate(s.disk.read_bps + s.disk.write_bps);
      diskChart.push([s.disk.read_bps / 1e6, s.disk.write_bps / 1e6]);
      (diskBody.querySelector("[data-r]") as HTMLElement).textContent = fmtRate(s.disk.read_bps);
      (diskBody.querySelector("[data-w]") as HTMLElement).textContent = fmtRate(s.disk.write_bps);
      if (frame % 20 === 1) {
        mountsWrap.innerHTML = "";
        for (const m of s.disk.mounts) {
          const pct = m.total > 0 ? (m.used / m.total) * 100 : 0;
          mountsWrap.append(
            h(
              "div",
              { class: "meter" },
              h("span", { class: "k", title: m.path }, m.path),
              h("div", { class: "prog" }, h("div", { style: { width: `${pct}%`, background: pct > 88 ? "linear-gradient(90deg,#7a1020,#ff3b5c)" : "" } })),
              h("span", { class: "v" }, `${pct.toFixed(0)}% of ${fmtBytes(m.total, 0)}`),
            ),
          );
        }
      }

      // network
      netChart.push([s.net.rx_bps / 1e6, s.net.tx_bps / 1e6]);
      rRx.v.textContent = fmtRate(s.net.rx_bps);
      rTx.v.textContent = fmtRate(s.net.tx_bps);

      // processes (refresh at 1 Hz)
      if (frame % 10 === 1) {
        procTbody.innerHTML = "";
        for (const p of s.top) {
          procTbody.append(
            h(
              "tr",
              {},
              h("td", {}, String(p.pid)),
              h("td", { title: p.name }, p.name),
              h("td", { class: "hi" }, p.cpu.toFixed(1)),
              h("td", {}, fmtBytes(p.mem)),
            ),
          );
        }
      }
    });
  }

  unmount() {
    this.unsub?.();
    this.charts.forEach((c) => c.destroy());
    window.removeEventListener("ap-freq-changed", this.freqHandler);
  }
}
