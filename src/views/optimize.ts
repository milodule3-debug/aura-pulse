// Optimization: power profiles, CPU boost, swappiness, cache drop.

import { call, onTelemetry } from "../lib/bridge";
import { h, icon, toast } from "../lib/ui";
import type { View } from "../app";

const PROFILES = [
  { id: "performance", icon: "bolt", name: "Performance", desc: "Max clocks, fans unleashed. For benchmarks and heavy builds." },
  { id: "balanced", icon: "scale", name: "Balanced", desc: "Adaptive clocks and power. The everyday default." },
  { id: "power-saver", icon: "leaf", name: "Power Saver", desc: "Lowest draw, cool and quiet. Squeeze the battery." },
];

export class OptimizeView implements View {
  private unsub: (() => void) | null = null;
  private info: any = null;

  async mount(root: HTMLElement) {
    const cards = new Map<string, HTMLElement>();
    const cardsWrap = h("div", { class: "profile-cards" });
    for (const p of PROFILES) {
      const card = h(
        "div",
        {
          class: "profile-card",
          onclick: async () => {
            try {
              await call("sysopt_set_profile", { profile: p.id });
              toast(`Profile → ${p.name}`);
              this.setActive(cards, p.id);
            } catch (e) {
              toast(String(e), true);
            }
          },
        },
        icon(p.icon, 30),
        h("div", { class: "name" }, p.name),
        h("div", { class: "desc" }, p.desc),
      );
      cards.set(p.id, card);
      cardsWrap.append(card);
    }

    // live power draw panel
    const watts = h("span", { class: "live" }, "—");
    const rows = h("div");

    root.append(
      h(
        "div",
        { class: "view-header" },
        h("h2", { html: "SYSTEM <b>OPTIMIZATION</b>" }),
        h("span", { class: "sub" }, "profiles · toggles · tuning"),
        h("div", { class: "spacer" }),
        h("div", { class: "readout accent" }, h("span", { class: "k" }, "TOTAL DRAW"), watts),
      ),
      cardsWrap,
      h(
        "section",
        { class: "panel" },
        h("div", { class: "panel-head" }, icon("settings"), h("span", { class: "t" }, "Tuning Console")),
        h("div", { class: "panel-body" }, rows),
      ),
    );

    this.unsub = onTelemetry((s) => {
      watts.textContent = `${(s.power.cpu_watts + s.gpu.watts).toFixed(1)} W`;
    });

    try {
      this.info = await call<any>("sysopt_get");
    } catch (e) {
      rows.append(h("div", { class: "bench-note" }, String(e)));
      return;
    }
    const info = this.info;
    this.setActive(cards, info.profile);

    const optRow = (t: string, d: string, control: HTMLElement) =>
      h("div", { class: "opt-row" }, h("div", { class: "info" }, h("div", { class: "t" }, t), h("div", { class: "d" }, d)), control);

    // CPU boost
    if (info.boost !== null && info.boost !== undefined) {
      const sw = h("button", { class: `switch${info.boost ? " on" : ""}` });
      sw.onclick = async () => {
        const target = !sw.classList.contains("on");
        try {
          await call("sysopt_set_boost", { on: target });
          sw.classList.toggle("on", target);
          toast(`CPU boost ${target ? "enabled" : "disabled"}`);
        } catch (e) {
          toast(String(e), true);
        }
      };
      rows.append(optRow("CPU Boost", "Turbo frequencies above base clock (needs authorization).", sw));
    }

    // swappiness
    const swapVal = h("span", { class: "val" }, String(info.swappiness));
    const slider = h("input", { type: "range", min: "0", max: "150", value: String(info.swappiness) }) as HTMLInputElement;
    slider.oninput = () => (swapVal.textContent = slider.value);
    const applyBtn = h(
      "button",
      {
        class: "btn ghost",
        onclick: async () => {
          try {
            await call("sysopt_set_swappiness", { value: Number(slider.value) });
            toast(`Swappiness → ${slider.value}`);
          } catch (e) {
            toast(String(e), true);
          }
        },
      },
      "Apply",
    );
    rows.append(
      optRow("Swappiness", "How aggressively memory pages move to swap. Lower keeps apps in RAM.",
        h("div", { style: { display: "flex", alignItems: "center", gap: "10px" } }, slider, swapVal, applyBtn)),
    );

    // drop caches
    rows.append(
      optRow("Drop Caches", "Flush pagecache, dentries and inodes. Frees RAM instantly; next file reads are colder.",
        h(
          "button",
          {
            class: "btn",
            onclick: async (e: Event) => {
              const b = e.currentTarget as HTMLButtonElement;
              b.disabled = true;
              try {
                await call("sysopt_drop_caches");
                toast("Caches dropped");
              } catch (err) {
                toast(String(err), true);
              }
              b.disabled = false;
            },
          },
          icon("refresh"),
          "Flush",
        )),
    );

    // readouts
    rows.append(
      optRow("CPU Governor", "Kernel frequency scaling policy currently in charge.",
        h("span", { class: "val", style: { minWidth: "auto" } }, info.governor || "—")),
    );
    if (info.epp) {
      rows.append(
        optRow("Energy-Performance Preference", "Hardware hint balancing speed vs efficiency (amd/intel pstate).",
          h("span", { class: "val", style: { minWidth: "auto" } }, info.epp)),
      );
    }
  }

  private setActive(cards: Map<string, HTMLElement>, id: string) {
    for (const [pid, el] of cards) {
      el.classList.toggle("active", pid === id);
      el.querySelector(".status-dot")?.remove();
      if (pid === id) el.append(h("div", { class: "status-dot" }));
    }
  }

  unmount() {
    this.unsub?.();
  }
}
