// AI Configuration Hub: provider credentials, models, endpoints.

import { call } from "../lib/bridge";
import { h, icon, toast } from "../lib/ui";
import type { View } from "../app";

export class SettingsView implements View {
  async mount(root: HTMLElement) {
    let cfg: any;
    try {
      cfg = await call<any>("ai_get_config");
    } catch (e) {
      root.append(h("div", { class: "bench-note" }, String(e)));
      return;
    }

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

    root.append(
      h(
        "div",
        { class: "view-header" },
        h("h2", { html: "AI CONFIGURATION <b>HUB</b>" }),
        h("span", { class: "sub" }, "keys stay local (~/.config/aura-pulse/ai.json, chmod 600) · calls go straight from the Rust core"),
      ),
      grid,
    );
  }

  unmount() {}
}
