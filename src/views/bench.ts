// Benchmarks: CPU / memory / disk suites + LLM performance estimation.

import { call, onEvent } from "../lib/bridge";
import { fmtNum } from "../lib/format";
import { h, icon, toast } from "../lib/ui";
import type { View } from "../app";

interface TestDef {
  id: string;
  icon: string;
  title: string;
  desc: string;
  render(result: any): HTMLElement;
}

const bigNum = (v: string, unit: string, label: string) =>
  h("div", { class: "big-num" }, h("span", { class: "v", html: `${v}<small> ${unit}</small>` }), h("span", { class: "k" }, label));

const TESTS: TestDef[] = [
  {
    id: "cpu",
    icon: "cpu",
    title: "CPU Throughput",
    desc: "Integer mixing workload, single core vs all cores.",
    render: (r) =>
      h(
        "div",
        { class: "bench-result" },
        bigNum(fmtNum(r.single_mops, 0), "Mops", "single core"),
        bigNum(fmtNum(r.multi_mops, 0), "Mops", `all ${r.threads} threads`),
        bigNum(r.scaling.toFixed(1), "×", "scaling"),
      ),
  },
  {
    id: "memory",
    icon: "mem",
    title: "Memory Bandwidth",
    desc: "Sequential read / write / copy over 256 MB buffers.",
    render: (r) =>
      h(
        "div",
        { class: "bench-result" },
        bigNum(r.gbps_read.toFixed(1), "GB/s", "read"),
        bigNum(r.gbps_write.toFixed(1), "GB/s", "write"),
        bigNum(r.gbps_copy.toFixed(1), "GB/s", "copy"),
      ),
  },
  {
    id: "disk",
    icon: "disk",
    title: "Disk Sequential",
    desc: "192 MB sequential write (fsync) and read-back.",
    render: (r) =>
      h(
        "div",
        {},
        h(
          "div",
          { class: "bench-result" },
          bigNum(fmtNum(r.write_mbps, 0), "MB/s", "write"),
          bigNum(fmtNum(r.read_mbps, 0), "MB/s", "read"),
        ),
        h("div", { class: "bench-note" }, r.note ?? ""),
      ),
  },
  {
    id: "llm",
    icon: "spark",
    title: "LLM Inference Estimate",
    desc: "Token-generation forecast from memory bandwidth; real run via Ollama when available.",
    render: (r) => {
      const tbl = h(
        "table",
        { class: "llm-table" },
        h("thead", {}, h("tr", {}, h("th", {}, "Model"), h("th", {}, "est. tokens/s"), h("th", {}, "fits RAM"))),
      );
      const tb = h("tbody");
      for (const e of r.estimates ?? []) {
        tb.append(
          h(
            "tr",
            {},
            h("td", {}, e.model),
            h("td", { class: "hi" }, e.tok_s.toFixed(1)),
            h("td", { class: e.fits_ram ? "" : "dim" }, e.fits_ram ? "yes" : "tight/no"),
          ),
        );
      }
      tbl.append(tb);
      const parts: HTMLElement[] = [
        h(
          "div",
          { class: "bench-result" },
          bigNum(r.bandwidth_gbps.toFixed(1), "GB/s", "measured bandwidth"),
          bigNum(r.effective_gbps.toFixed(1), "GB/s", "effective (×0.72)"),
        ),
        tbl,
      ];
      if (r.ollama) {
        parts.push(
          h("div", { class: "ai-result-label", style: { marginTop: "12px" } }, "REAL RUN — OLLAMA"),
          h(
            "div",
            { class: "bench-result" },
            bigNum(r.ollama.tok_s.toFixed(1), "tok/s", r.ollama.model),
            bigNum(r.ollama.prompt_tok_s.toFixed(0), "tok/s", "prompt eval"),
          ),
        );
      } else {
        parts.push(h("div", { class: "bench-note" }, "Ollama not detected on :11434 — start it for a real measurement."));
      }
      return h("div", {}, ...parts);
    },
  },
];

export class BenchView implements View {
  private unsub: (() => void) | null = null;
  private cards = new Map<string, { prog: HTMLElement; label: HTMLElement; out: HTMLElement; btn: HTMLButtonElement }>();
  private running = false;

  mount(root: HTMLElement) {
    const grid = h("div", { class: "bench-grid" });

    for (const t of TESTS) {
      const prog = h("div", { class: "prog", style: { margin: "10px 0 6px" } }, h("div", { style: { width: "0%" } }));
      const label = h("div", { class: "bench-note" }, "idle");
      const out = h("div");
      const btn = h(
        "button",
        { class: "btn", onclick: () => this.run(t) },
        icon("play"),
        "Run",
      ) as HTMLButtonElement;

      this.cards.set(t.id, { prog, label, out, btn });
      grid.append(
        h(
          "section",
          { class: "panel" },
          h("div", { class: "panel-head" }, icon(t.icon), h("span", { class: "t" }, t.title), h("div", { class: "spacer" }), btn),
          h("div", { class: "panel-body" }, h("div", { class: "bench-note" }, t.desc), prog, label, out),
        ),
      );
    }

    const runAll = h(
      "button",
      {
        class: "btn",
        onclick: async () => {
          for (const t of TESTS) await this.run(t);
        },
      },
      icon("zap"),
      "Run Full Suite",
    );

    root.append(
      h(
        "div",
        { class: "view-header" },
        h("h2", { html: "HARDWARE <b>BENCHMARKS</b>" }),
        h("span", { class: "sub" }, "throughput · bandwidth · AI capability"),
        h("div", { class: "spacer" }),
        runAll,
      ),
      grid,
    );

    this.unsub = onEvent<{ test: string; pct: number; label: string }>("bench_progress", (p) => {
      const c = this.cards.get(p.test);
      if (!c) return;
      (c.prog.firstElementChild as HTMLElement).style.width = `${p.pct}%`;
      c.label.textContent = p.label;
    });
  }

  private async run(t: TestDef) {
    if (this.running) return;
    const c = this.cards.get(t.id)!;
    this.running = true;
    c.btn.disabled = true;
    c.label.textContent = "running…";
    (c.prog.firstElementChild as HTMLElement).style.width = "5%";
    try {
      const result = await call<any>("bench_run", { test: t.id });
      c.out.innerHTML = "";
      c.out.append(t.render(result));
      c.label.textContent = "complete";
      (c.prog.firstElementChild as HTMLElement).style.width = "100%";
    } catch (e) {
      c.label.textContent = String(e);
      toast(String(e), true);
    }
    c.btn.disabled = false;
    this.running = false;
  }

  unmount() {
    this.unsub?.();
  }
}
