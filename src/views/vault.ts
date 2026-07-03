// The Vault: AI-enhanced clipboard history.

import { call, ClipFull, ClipRow, onEvent } from "../lib/bridge";
import { fmtAgo, fmtBytes } from "../lib/format";
import { confirmDialog, h, icon, toast } from "../lib/ui";
import type { View } from "../app";

const KIND_ICON: Record<string, string> = {
  text: "text", url: "link", code: "code", json: "braces", csv: "braces",
  email: "mail", color: "palette", image: "image", audio: "audio",
};

const FILTERS = [
  { id: "all", label: "All" },
  { id: "text", label: "Text" },
  { id: "url", label: "Links" },
  { id: "code", label: "Code" },
  { id: "json", label: "JSON" },
  { id: "color", label: "Colors" },
  { id: "image", label: "Images" },
  { id: "pinned", label: "★ Pinned" },
];

const AI_TASKS: { id: string; label: string; imageOnly?: boolean }[] = [
  { id: "summarize", label: "Summarize" },
  { id: "ocr", label: "OCR", imageOnly: true },
  { id: "describe", label: "Describe", imageOnly: true },
  { id: "markdown", label: "→ Markdown" },
  { id: "design", label: "→ Design JSON" },
];

export class VaultView implements View {
  private root!: HTMLElement;
  private grid!: HTMLElement;
  private footer!: HTMLElement;
  private query = "";
  private filter = "all";
  private drawer: HTMLElement | null = null;
  private unsub: (() => void) | null = null;
  private searchTimer = 0;

  mount(root: HTMLElement) {
    this.root = root;

    const search = h("input", {
      type: "search",
      placeholder: "Search 5,000-clip history…",
      oninput: (e: Event) => {
        this.query = (e.target as HTMLInputElement).value;
        clearTimeout(this.searchTimer);
        this.searchTimer = window.setTimeout(() => this.refresh(), 220);
      },
    });

    const wipeBtn = h(
      "button",
      {
        class: "btn danger",
        onclick: async () => {
          if (await confirmDialog("Wipe The Vault", "This permanently deletes every clip in the local database, including pinned ones. There is no undo.", "WIPE ALL")) {
            await call("vault_wipe");
            toast("Vault wiped");
            this.refresh();
          }
        },
      },
      icon("trash"),
      "Wipe All",
    );

    const chips = h("div", { class: "vault-chips" });
    for (const f of FILTERS) {
      const c = h(
        "button",
        {
          class: `chip${this.filter === f.id ? " active" : ""}`,
          onclick: () => {
            this.filter = f.id;
            chips.querySelectorAll(".chip").forEach((el) => el.classList.remove("active"));
            c.classList.add("active");
            this.refresh();
          },
        },
        f.label,
      );
      chips.append(c);
    }

    this.grid = h("div", { class: "clip-grid" });
    this.footer = h("div", { class: "vault-footer" });

    root.append(
      h(
        "div",
        { class: "view-header" },
        h("h2", { html: "THE <b>VAULT</b>" }),
        h("span", { class: "sub" }, "everything you copy becomes searchable"),
        h("div", { class: "spacer" }),
        wipeBtn,
      ),
      h("div", { class: "vault-toolbar" }, search),
      chips,
      this.buildCaptureBar(),
      this.grid,
      this.footer,
    );

    this.refresh();
    this.unsub = onEvent("vault_changed", () => this.refresh());
  }

  private buildCaptureBar(): HTMLElement {
    const body = h("div", { class: "capture-body" });
    const tabs = h("div", { class: "capture-tabs" });
    const modes: Record<string, () => void> = {
      Text: () => {
        const ta = h("textarea", { placeholder: "Paste or type — added straight to the vault…" }) as HTMLTextAreaElement;
        body.append(
          ta,
          h(
            "button",
            {
              class: "btn",
              style: { alignSelf: "flex-end" },
              onclick: async () => {
                if (!ta.value.trim()) return;
                await call("vault_add_text", { content: ta.value });
                ta.value = "";
                toast("Clip stored");
                this.refresh();
              },
            },
            icon("send"),
            "Store",
          ),
        );
      },
      Image: () => {
        const dz = h("div", { class: "dropzone" }, "Drop an image here — OCR, description and design extraction become available");
        this.wireDrop(dz, "image");
        body.append(dz);
      },
      Audio: () => {
        const dz = h("div", { class: "dropzone" }, "Drop an audio file — stored in the vault; transcription arrives with a provider that supports audio input");
        this.wireDrop(dz, "audio");
        body.append(dz);
      },
    };
    let first = true;
    for (const name of Object.keys(modes)) {
      const b = h(
        "button",
        {
          class: first ? "active" : "",
          onclick: () => {
            tabs.querySelectorAll("button").forEach((x) => x.classList.remove("active"));
            b.classList.add("active");
            body.innerHTML = "";
            modes[name]();
          },
        },
        name,
      );
      tabs.append(b);
      first = false;
    }
    modes.Text();
    return h("div", { class: "capture-bar" }, tabs, body);
  }

  private wireDrop(dz: HTMLElement, kind: "image" | "audio") {
    dz.ondragover = (e) => {
      e.preventDefault();
      dz.classList.add("over");
    };
    dz.ondragleave = () => dz.classList.remove("over");
    dz.ondrop = async (e) => {
      e.preventDefault();
      dz.classList.remove("over");
      const file = e.dataTransfer?.files?.[0];
      if (!file) return;
      if (kind === "image" && !file.type.startsWith("image/")) return toast("Not an image file", true);
      const buf = await file.arrayBuffer();
      const b64 = btoa(new Uint8Array(buf).reduce((s, b) => s + String.fromCharCode(b), ""));
      if (kind === "image") {
        await call("vault_add_image", { dataB64: b64 });
        toast("Image stored in vault");
      } else {
        // audio stored as text stub with filename until multimodal audio lands
        await call("vault_add_text", { content: `[audio] ${file.name} (${fmtBytes(file.size)})` });
        toast("Audio reference stored");
      }
      this.refresh();
    };
  }

  private async refresh() {
    try {
      const rows = await call<ClipRow[]>("vault_list", {
        args: {
          query: this.query || null,
          kind: this.filter === "pinned" ? null : this.filter,
          pinned_only: this.filter === "pinned",
          limit: 80,
        },
      });
      this.grid.innerHTML = "";
      if (!rows.length) {
        this.grid.append(
          h(
            "div",
            { style: { gridColumn: "1/-1", textAlign: "center", padding: "60px 0", color: "var(--text-low)", letterSpacing: ".1em" } },
            "VAULT EMPTY — copy something, it will materialize here",
          ),
        );
      }
      for (const c of rows) this.grid.append(this.card(c));

      const stats = await call<any>("vault_stats");
      this.footer.innerHTML = "";
      this.footer.append(
        h("span", {}, h("b", {}, String(stats.total)), ` / 5000 clips`),
        h("span", {}, h("b", {}, String(stats.pinned)), " pinned"),
        h("span", {}, h("b", {}, fmtBytes(stats.db_bytes)), " on disk"),
        ...stats.by_kind.slice(0, 5).map(([k, n]: [string, number]) => h("span", {}, `${k}: `, h("b", {}, String(n)))),
      );
    } catch (e) {
      toast(String(e), true);
    }
  }

  private card(c: ClipRow): HTMLElement {
    const actions = h(
      "div",
      { class: "clip-actions" },
      h(
        "button",
        {
          class: `pin${c.pinned ? " on" : ""}`,
          title: c.pinned ? "Unpin" : "Pin",
          onclick: async (e: Event) => {
            e.stopPropagation();
            await call("vault_pin", { id: c.id, pinned: !c.pinned });
            this.refresh();
          },
        },
        icon("pin"),
      ),
      h(
        "button",
        {
          title: "Copy to clipboard",
          onclick: async (e: Event) => {
            e.stopPropagation();
            await call("vault_copy", { id: c.id });
            toast("Copied to system clipboard");
          },
        },
        icon("copy"),
      ),
      h(
        "button",
        {
          title: "Delete",
          onclick: async (e: Event) => {
            e.stopPropagation();
            await call("vault_delete", { id: c.id });
            this.refresh();
          },
        },
        icon("trash"),
      ),
    );

    let preview: HTMLElement;
    if (c.kind === "image" && c.thumb) {
      preview = h("div", { class: "clip-preview" }, h("img", { src: c.thumb, alt: "clip" }));
    } else if (c.kind === "color" && c.content) {
      preview = h(
        "div",
        {},
        h("div", { class: "clip-swatch", style: { background: c.content.trim() } }),
        h("div", { class: "clip-preview", style: { marginTop: "6px" } }, c.content),
      );
    } else {
      preview = h("div", { class: "clip-preview" }, c.content ?? "");
    }

    const meta = h("div", { class: "clip-meta" });
    if (c.tags) for (const t of c.tags.split(",").slice(0, 4)) meta.append(h("span", { class: "clip-tag" }, t.trim()));

    return h(
      "article",
      { class: `clip-card${c.pinned ? " pinned" : ""}`, onclick: () => this.openDrawer(c.id) },
      h(
        "div",
        { class: "clip-head" },
        h("span", { class: `kind-badge kind-${c.kind}` }, icon(KIND_ICON[c.kind] ?? "text"), c.kind),
        c.has_ai ? h("span", { class: "ai-dot", title: "AI-enriched" }) : null,
        h("span", { class: "clip-time" }, fmtAgo(c.created_at)),
        actions,
      ),
      c.title ? h("div", { class: "clip-title" }, c.title) : null,
      preview,
      c.summary ? h("div", { class: "clip-preview", style: { marginTop: "6px", color: "var(--cyan-dim)" } }, c.summary) : null,
      meta,
    );
  }

  private async openDrawer(id: number) {
    this.closeDrawer();
    let clip: ClipFull;
    try {
      clip = await call<ClipFull>("vault_get", { id });
    } catch (e) {
      return toast(String(e), true);
    }

    const results = h("div");
    const renderResults = (c: ClipFull) => {
      results.innerHTML = "";
      const add = (label: string, val: string | null) => {
        if (!val) return;
        results.append(h("div", { class: "ai-result-label" }, label), h("div", { class: "ai-result" }, val));
      };
      if (c.title || c.summary) add("Summary", [c.title, c.summary, c.tags].filter(Boolean).join("\n"));
      add("OCR", c.ocr);
      add("Description", c.description);
      add("Markdown", c.markdown);
      add("Design JSON", c.design_json);
    };
    renderResults(clip);

    const aiBtns = h("div", { class: "ai-actions" });
    for (const t of AI_TASKS) {
      if (t.imageOnly && clip.kind !== "image") continue;
      const b = h(
        "button",
        {
          class: "btn",
          onclick: async () => {
            b.setAttribute("disabled", "");
            b.textContent = "RUNNING…";
            try {
              await call("ai_run", { clipId: id, task: t.id });
              const fresh = await call<ClipFull>("vault_get", { id });
              renderResults(fresh);
              toast(`${t.label} complete`);
            } catch (e) {
              toast(String(e), true);
            }
            b.removeAttribute("disabled");
            b.innerHTML = "";
            b.append(icon("spark"), t.label);
          },
        },
        icon("spark"),
        t.label,
      );
      aiBtns.append(b);
    }

    const askInput = h("input", { placeholder: "Ask AI about this clip…" }) as HTMLInputElement;
    const askOut = h("div");
    const ask = async () => {
      const q = askInput.value.trim();
      if (!q) return;
      askOut.innerHTML = "";
      askOut.append(h("div", { class: "ai-result-label" }, "Aura"), h("div", { class: "ai-result" }, "…"));
      try {
        const reply = await call<string>("ai_chat", { prompt: q, clipId: id });
        (askOut.lastElementChild as HTMLElement).textContent = reply;
      } catch (e) {
        (askOut.lastElementChild as HTMLElement).textContent = String(e);
      }
    };
    askInput.onkeydown = (e) => e.key === "Enter" && ask();

    const content =
      clip.kind === "image" && clip.image
        ? h("div", { class: "drawer-content" }, h("img", { src: clip.image, alt: "clip" }))
        : h("div", { class: "drawer-content" }, clip.content ?? "");

    this.drawer = h(
      "div",
      { class: "drawer" },
      h(
        "div",
        { class: "drawer-head" },
        h("span", { class: `kind-badge kind-${clip.kind}` }, icon(KIND_ICON[clip.kind] ?? "text"), clip.kind),
        h("span", { class: "clip-time" }, fmtAgo(clip.created_at), clip.width ? ` · ${clip.width}×${clip.height}` : ""),
        h("div", { class: "spacer", style: { flex: "1" } }),
        h(
          "button",
          {
            class: "icon-btn",
            onclick: async () => {
              await call("vault_copy", { id });
              toast("Copied");
            },
          },
          icon("copy"),
        ),
        h("button", { class: "icon-btn", onclick: () => this.closeDrawer() }, icon("x")),
      ),
      h("div", { class: "drawer-body" }, content, aiBtns, results, askOut),
      h("div", { class: "ask-row" }, askInput, h("button", { class: "btn", onclick: ask }, icon("send"), "Ask")),
    );
    document.body.append(this.drawer);
  }

  private closeDrawer() {
    this.drawer?.remove();
    this.drawer = null;
  }

  unmount() {
    this.closeDrawer();
    this.unsub?.();
  }
}
