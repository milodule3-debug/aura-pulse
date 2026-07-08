// AI Configuration Hub: provider credentials, models, endpoints.
// + Display settings: theme picker, telemetry frequency, clipboard notes.

import { call } from "../lib/bridge";
import { h, icon, toast } from "../lib/ui";
import type { View } from "../app";

// ---------- themes ----------

interface ThemeDef {
  id: string;
  label: string;
  preview: string;   // CSS gradient for the swatch preview
}

const THEMES: ThemeDef[] = [
  {
    id: "neon",
    label: "Neon",
    preview: "linear-gradient(135deg, #04070c 0%, #0a1420 40%, #00e5ff22 100%)",
  },
  {
    id: "wireframe",
    label: "Wireframe",
    preview: "linear-gradient(135deg, #08090e 0%, #10162a 40%, #4dd9f044 100%)",
  },
  {
    id: "dracula",
    label: "Dracula",
    preview: "linear-gradient(135deg, #282a36 0%, #343746 40%, #bd93f944 100%)",
  },
];

// ---------- telemetry frequency ----------

/** Returns a human-readable label for a throttle divisor (1 = 10Hz, 5 = 2Hz). */
function freqLabel(div: number): string {
  const hz = 10 / div;
  return hz >= 1 ? `${hz.toFixed(hz % 1 ? 1 : 0)} Hz` : `${(1 / hz).toFixed(0)}s`;
}

// ---------- view ----------

export class SettingsView implements View {
  async mount(root: HTMLElement) {
    let cfg: any;
    try {
      cfg = await call<any>("ai_get_config");
    } catch (e) {
      root.append(h("div", { class: "bench-note" }, String(e)));
      return;
    }

    // ═══════ display section ═══════

    // theme picker
    const currentTheme = localStorage.getItem("ap-theme") || "neon";
    const themeRow = h("div", { class: "theme-picker" });
    const swatches = new Map<string, HTMLElement>();
    for (const t of THEMES) {
      const swatch = h(
        "div",
        {
          class: `theme-swatch${t.id === currentTheme ? " active" : ""}`,
          onclick: () => {
            document.body.className = t.id === "neon" ? "" : `theme-${t.id}`;
            localStorage.setItem("ap-theme", t.id);
            for (const [, el] of swatches) el.classList.remove("active");
            swatch.classList.add("active");
            // notify charts they may need to adjust
            window.dispatchEvent(new CustomEvent("ap-theme-changed", { detail: t.id }));
            toast(`Theme → ${t.label}`);
          },
        },
        h("div", { class: "swatch-preview", style: { background: t.preview } }),
        h("div", { class: "swatch-name" }, t.label),
      );
      swatches.set(t.id, swatch);
      themeRow.append(swatch);
    }

    // telemetry frequency slider
    const savedDiv = Number(localStorage.getItem("ap-telem-div")) || 1;
    const freqVal = h("span", { class: "freq-val" }, freqLabel(savedDiv));
    const freqSlider = h("input", {
      type: "range",
      min: "1",
      max: "50",
      value: String(savedDiv),
    }) as HTMLInputElement;
    freqSlider.oninput = () => {
      const d = Number(freqSlider.value);
      freqVal.textContent = freqLabel(d);
    };
    freqSlider.onchange = () => {
      const d = Number(freqSlider.value);
      localStorage.setItem("ap-telem-div", String(d));
      window.dispatchEvent(new CustomEvent("ap-freq-changed", { detail: d }));
      toast(`Telemetry → ${freqLabel(d)}`);
    };

    // clipboard note
    const clipNote = h(
      "div",
      { class: "bench-note", style: { maxWidth: "560px", lineHeight: "1.6" } },
      "Clipboard monitoring runs in the Rust backend via ",
      h("b", {}, "wl-paste --watch"),
      " on Wayland or ",
      h("b", {}, "xclip"),
      " polling on X11. If copies are missed, ensure the correct tool is installed and your compositor is forwarding clipboard events. On Wayland, ",
      h("b", {}, "wl-clipboard"),
      " is required.",
    );

    // ═══════ AI providers section ═══════

    const grid = h("div", { class: "prov-grid" });
    const cardEls = new Map<string, HTMLElement>();

    const save = async () => {
      try {
        await call("ai_set_config", { cfg });
        window.dispatchEvent(new Event("ai-config-changed"));
      } catch (e) {
        toast(String(e), true);
      }
    };

    for (const [key, p] of Object.entries<any>(cfg.providers)) {
      const field = (label: string, prop: string, type = "text", placeholder = "") => {
        const input = h("input", { type, value: p[prop] ?? "", placeholder, spellcheck: "false" }) as HTMLInputElement;
        input.onchange = () => {
          p[prop] = input.value.trim();
          save();
        };
        return h("div", { class: "prov-field" }, h("label", {}, label), input);
      };

      const testOut = h("span", { class: "test-out" }, "");
      const testBtn = h(
        "button",
        {
          class: "btn ghost",
          onclick: async (e: Event) => {
            const b = e.currentTarget as HTMLButtonElement;
            b.disabled = true;
            testOut.className = "test-out";
            testOut.textContent = "linking…";
            try {
              const r = await call<any>("ai_test", { provider: key });
              testOut.className = `test-out ${r.ok ? "ok" : "err"}`;
              testOut.textContent = r.ok ? `✓ ${r.latency_ms}ms — ${r.message}` : `✗ ${r.message}`;
            } catch (err) {
              testOut.className = "test-out err";
              testOut.textContent = String(err);
            }
            b.disabled = false;
          },
        },
        "Test Link",
      );

      const activate = h(
        "button",
        {
          class: "btn",
          onclick: () => {
            cfg.active = key;
            save();
            for (const [k, el] of cardEls) el.classList.toggle("active-provider", k === key);
            toast(`AI core → ${p.label}`);
          },
        },
        icon("zap"),
        "Activate",
      );

      const card = h(
        "div",
        { class: `prov-card${cfg.active === key ? " active-provider" : ""}` },
        h(
          "div",
          { class: "prov-head" },
          h("span", { class: "name" }, p.label),
          h("span", { class: "kind-tag" }, p.kind),
          h("div", { class: "spacer", style: { flex: "1" } }),
        ),
        field("API Key", "api_key", "password", key === "ollama" || key === "lmstudio" ? "not required" : "sk-…"),
        field("Model", "model"),
        field("Base URL", "base_url", "text", "http://host:port/v1"),
        h("div", { class: "prov-foot" }, activate, testBtn, testOut),
      );
      cardEls.set(key, card);
      grid.append(card);
    }

    // ═══════ assemble ═══════

    root.append(
      h(
        "div",
        { class: "view-header" },
        h("h2", { html: "AI CONFIGURATION <b>HUB</b>" }),
        h("span", { class: "sub" }, "keys stay local (~/.config/aura-pulse/ai.json, chmod 600) · calls go straight from the Rust core"),
      ),

      // display settings — all three side by side
      h("div", { class: "settings-row" },
        h("div", { class: "settings-section" },
          h("div", { class: "settings-section-title" }, "Display"),
          themeRow,
        ),

        h("div", { class: "settings-section" },
          h("div", { class: "settings-section-title" }, "Telemetry Frequency"),
          h("div", { class: "bench-note", style: { marginBottom: "10px" } },
            "Lower frequency reduces CPU usage from chart rendering. Default is 10 Hz; slide right to throttle."),
          h("div", { class: "freq-slider-wrap" },
            h("span", { class: "freq-label" }, "10 Hz"),
            freqSlider,
            h("span", { class: "freq-label" }, "0.2 Hz"),
            freqVal,
          ),
        ),

        h("div", { class: "settings-section" },
          h("div", { class: "settings-section-title" }, "Clipboard Monitoring"),
          clipNote,
        ),
      ),

      // AI providers
      h("div", { class: "settings-section" },
        h("div", { class: "settings-section-title" }, "AI Providers"),
        grid,
      ),
    );
  }

  unmount() {}
}
