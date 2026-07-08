// The Vault: AI-enhanced clipboard history.

import { call, ClipFull, ClipRow, onDragDrop, onEvent } from "../lib/bridge";
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
  { id: "audio", label: "Audio" },
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
  private dragUnsub: (() => void) | null = null;
  private activeDz: { el: HTMLElement; kind: "image" | "audio" } | null = null;
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

    // Native drops (Tauri intercepts HTML5 drag events; files arrive as paths).
    this.dragUnsub = onDragDrop(async (e) => {
      const dz = this.activeDz;
      if (!dz || !dz.el.isConnected) return;
      if (e.type === "leave") return void dz.el.classList.remove("over");
      const r = dz.el.getBoundingClientRect();
      const inside = e.x >= r.left && e.x <= r.right && e.y >= r.top && e.y <= r.bottom;
      if (e.type === "over") return void dz.el.classList.toggle("over", inside);
      dz.el.classList.remove("over");
      if (!inside || !e.paths.length) return;
      try {
        await call("vault_add_path", { path: e.paths[0], kind: dz.kind });
        toast(dz.kind === "image" ? "Image stored in vault" : "Audio reference stored");
        this.refresh();
      } catch (err) {
        toast(String(err), true);
      }
    });
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
        const dz = h("div", { class: "dropzone" }, "Drop an image here or click to browse — OCR, description and design extraction become available");
        this.wireDrop(dz, "image");
        body.append(dz);
      },
      Audio: () => {
        const dz = h("div", { class: "dropzone" }, "Drop an audio file or click to browse — stored in the vault; transcription arrives with a provider that supports audio input");
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
            this.activeDz = null;
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

  private async fileToB64(file: File): Promise<string> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(bin);
  }

  private async ingestFile(file: File, kind: "image" | "audio") {
    if (kind === "image" && !file.type.startsWith("image/")) return toast("Not an image file", true);
    if (kind === "audio" && !file.type.startsWith("audio/") && !/\.(mp3|wav|ogg|flac|m4a|opus|aac|wma)$/i.test(file.name)) {
      return toast("Not an audio file", true);
    }
    if (file.size > 25_000_000) return toast("File too large (max 25 MB)", true);
    try {
      let id: number;
      if (kind === "image") {
        id = await call<number>("vault_add_image", { dataB64: await this.fileToB64(file) });
        toast("Image stored in vault");
      } else {
        id = await call<number>("vault_add_audio", { dataB64: await this.fileToB64(file), name: file.name });
        toast("Audio stored in vault");
      }
      this.refresh();
      this.autoEnrich(id, kind);
    } catch (e) {
      toast(String(e), true);
    }
  }

  /// Auto-run AI on freshly ingested files when a provider is configured:
  /// images get summarized, audio gets transcribed/described.
  private async autoEnrich(id: number, kind: "image" | "audio") {
    if (!id) return;
    try {
      const cfg = await call<any>("ai_get_config");
      const usable = (p: any) => !!p && (p.api_key || /127\.0\.0\.1|localhost/.test(p.base_url ?? ""));
      const active = cfg.providers?.[cfg.active];
      const ok = kind === "image" ? usable(active) : usable(active) || usable(cfg.providers?.gemini);
      if (!ok) return; // no LLM configured — store silently, enrich later by hand
      toast(kind === "image" ? "AI summarizing…" : "Transcribing…");
      if (kind === "image") await call("ai_run", { clipId: id, task: "summarize" });
      else await call("ai_transcribe", { clipId: id });
      toast(kind === "image" ? "AI summary ready" : "Transcript ready");
      this.refresh();
    } catch (e) {
      toast(String(e), true);
    }
  }

  private wireDrop(dz: HTMLElement, kind: "image" | "audio") {
    this.activeDz = { el: dz, kind };
    // Primary path: HTML5 drag events — the window is created with
    // dragDropEnabled: false so webkit delivers them (native interception
    // otherwise swallows drops and its own events don't fire on Wayland).
    // The onDragDrop listener in mount() stays as a fallback for platforms
    // where the config flips back.
    const picker = h("input", {
      type: "file",
      accept: kind === "image" ? "image/*" : "audio/*",
      style: { display: "none" },
    }) as HTMLInputElement;
    picker.onchange = () => {
      const f = picker.files?.[0];
      if (f) this.ingestFile(f, kind);
      picker.value = "";
    };
    dz.append(picker);
    dz.onclick = () => picker.click();
    dz.ondragover = (e) => {
      e.preventDefault();
      dz.classList.add("over");
    };
    dz.ondragleave = () => dz.classList.remove("over");
    dz.ondrop = (e) => {
      e.preventDefault();
      dz.classList.remove("over");
      const file = e.dataTransfer?.files?.[0];
      if (file) this.ingestFile(file, kind);
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
      h(
        "button",
        {
          title: "Save as file",
          onclick: async (e: Event) => {
            e.stopPropagation();
            try {
              const path = await call<string>("vault_save_as", { id: c.id });
              if (path) toast(`Saved: ${path}`);
            } catch (err) {
              if (!String(err).includes("cancelled")) toast(String(err), true);
            }
          },
        },
        icon("download"),
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
      add(c.kind === "audio" ? "Transcript" : "OCR", c.ocr);
      add("Description", c.description);
      add("Markdown", c.markdown);
      add("Design JSON", c.design_json);
    };
    renderResults(clip);

    const aiBtns = h("div", { class: "ai-actions" });
    if (clip.kind === "audio") {
      for (const m of [
        { mode: "transcribe", label: "Transcribe", done: "Transcription complete" },
        { mode: "describe", label: "Describe", done: "Description complete" },
      ]) {
        const b = h(
          "button",
          {
            class: "btn",
            onclick: async () => {
              b.setAttribute("disabled", "");
              b.textContent = "RUNNING…";
              try {
                await call("ai_transcribe", { clipId: id, mode: m.mode });
                const fresh = await call<ClipFull>("vault_get", { id });
                renderResults(fresh);
                toast(m.done);
              } catch (e) {
                toast(String(e), true);
              }
              b.removeAttribute("disabled");
              b.innerHTML = "";
              b.append(icon("spark"), m.label);
            },
          },
          icon("spark"),
          m.label,
        );
        aiBtns.append(b);
      }
    }
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
    askInput.onkeydown = (e) => {
      if (e.key === "Enter") ask();
    };

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
            title: "Copy to clipboard",
            onclick: async () => {
              await call("vault_copy", { id });
              toast("Copied");
            },
          },
          icon("copy"),
        ),
        h(
          "button",
          {
            class: "icon-btn",
            title: "Save as file",
            onclick: async () => {
              try {
                const path = await call<string>("vault_save_as", { id });
                if (path) toast(`Saved: ${path}`);
              } catch (e) {
                if (!String(e).includes("cancelled")) toast(String(e), true);
              }
            },
          },
          icon("download"),
        ),
        h("button", { class: "icon-btn", onclick: () => this.closeDrawer() }, icon("x")),
      ),
      h("div", { class: "drawer-body" }, content, aiBtns, results, askOut),
      h("div", { class: "ask-row" }, askInput, h("button", { class: "btn", onclick: ask }, icon("send"), "Ask")),
    );
    document.body.append(this.drawer);

    const body = this.drawer.querySelector(".drawer-body") as HTMLElement;
    if (body) body.scrollTop = body.scrollHeight;
    askInput.focus();
  }

  private closeDrawer() {
    this.drawer?.remove();
    this.drawer = null;
  }

  unmount() {
    this.closeDrawer();
    this.unsub?.();
    this.dragUnsub?.();
    this.activeDz = null;
  }
}
