// Dev-only integration selftest. Runs inside the real Tauri app when
// public/selftest.flag exists (dev server only); results render on-screen
// so a screenshot documents the pass/fail state.

import { call, isTauri, latest } from "./lib/bridge";

export async function maybeRunSelftest() {
  if (!import.meta.env.DEV) return;
  try {
    const r = await fetch("/selftest.flag");
    if (!r.ok) return;
  } catch {
    return;
  }

  const panel = document.createElement("div");
  panel.style.cssText =
    "position:fixed;left:50%;top:70px;transform:translateX(-50%);z-index:999;background:rgba(4,8,14,.96);" +
    "border:1px solid #00e5ff;border-radius:8px;padding:14px 20px;font:12px 'JetBrains Mono Variable',monospace;" +
    "color:#d9f4ff;min-width:520px;box-shadow:0 0 30px rgba(0,229,255,.3)";
  panel.innerHTML = `<b style="color:#00e5ff">AURA SELFTEST — ${isTauri ? "NATIVE" : "BROWSER-MOCK"}</b><br>`;
  document.body.append(panel);
  const log = (line: string) => {
    panel.innerHTML += line + "<br>";
    console.log("[SELFTEST]", line.replace(/<[^>]+>/g, ""));
  };
  const ok = (name: string, detail: string) => log(`<span style="color:#47ffa0">✓</span> ${name} — ${detail}`);
  const fail = (name: string, e: any) => log(`<span style="color:#ff3b5c">✗</span> ${name} — ${String(e).slice(0, 140)}`);

  // 1. telemetry
  try {
    const s = await call<any>("telemetry_snapshot");
    if (!s.cpu.cores.length) throw new Error("no cores");
    ok("telemetry", `${s.cpu.cores.length} cores, cpu ${s.cpu.total.toFixed(1)}%, temp ${s.cpu.temp.toFixed(0)}°C, uptime ${s.sys.uptime}s`);
  } catch (e) {
    fail("telemetry", e);
  }

  // 2. vault round-trip
  try {
    const marker = `AURA SELFTEST ${Date.now()}`;
    await call("vault_add_text", { content: marker });
    const rows = await call<any[]>("vault_list", { args: { query: marker, limit: 5 } });
    if (!rows.length) throw new Error("clip not found after insert");
    await call("vault_copy", { id: rows[0].id });
    await call("vault_delete", { id: rows[0].id });
    ok("vault", `insert→search→copy→delete on id ${rows[0].id} (kind ${rows[0].kind})`);
  } catch (e) {
    fail("vault", e);
  }

  // 3. sysopt
  try {
    const o = await call<any>("sysopt_get");
    ok("sysopt", `profile=${o.profile} governor=${o.governor} boost=${o.boost} swappiness=${o.swappiness}`);
  } catch (e) {
    fail("sysopt", e);
  }

  // 4. AI config + link test against the active provider
  try {
    const cfg = await call<any>("ai_get_config");
    ok("ai_config", `active=${cfg.active}, ${Object.keys(cfg.providers).length} providers`);
    const t = await call<any>("ai_test", { provider: cfg.active });
    (t.ok ? ok : fail)(`ai_link(${cfg.active})`, `${t.latency_ms}ms — ${t.message}`);
  } catch (e) {
    fail("ai_config", e);
  }

  // 5. native drop ingestion (vault_add_path) — dev CWD is src-tauri/
  if (isTauri) {
    try {
      await call("vault_add_path", { path: "icons/32x32.png", kind: "image" });
      await new Promise((r) => setTimeout(r, 500)); // let external observers see the row
      const imgs = await call<any[]>("vault_list", { args: { kind: "image", limit: 10 } });
      const fresh = imgs.find((c) => !c.pinned && Date.now() - c.created_at < 15000);
      if (!fresh) throw new Error("dropped image not found in vault");
      await call("vault_delete", { id: fresh.id });
      ok("vault_add_path", `ingested icons/32x32.png as image clip id ${fresh.id}, cleaned up`);
    } catch (e) {
      fail("vault_add_path", e);
    }
  }

  // 5b. picker/drop ingestion: raw-base64 JPEG through vault_add_image
  if (isTauri) {
    try {
      const cv = document.createElement("canvas");
      cv.width = cv.height = 24;
      const cx = cv.getContext("2d")!;
      cx.fillStyle = "#00e5ff";
      cx.fillRect(0, 0, 24, 24);
      const b64 = cv.toDataURL("image/jpeg").split(",")[1]; // raw base64, no data-URL prefix
      const id = await call<number>("vault_add_image", { dataB64: b64 });
      if (!id) throw new Error("no clip id returned");
      await new Promise((r) => setTimeout(r, 400)); // let external observers see the row
      await call("vault_delete", { id });
      ok("vault_add_image(b64 jpeg)", `decoded + stored as clip id ${id}, cleaned up`);
    } catch (e) {
      fail("vault_add_image(b64 jpeg)", e);
    }
  }

  // 6. quick memory benchmark
  try {
    const b = await call<any>("bench_run", { test: "memory" });
    ok("bench_memory", `read ${b.gbps_read.toFixed(1)} / write ${b.gbps_write.toFixed(1)} / copy ${b.gbps_copy.toFixed(1)} GB/s`);
  } catch (e) {
    fail("bench_memory", e);
  }

  log("<i style='color:#86aec4'>done — telemetry snapshot in store: " + (latest.snap ? "yes" : "no") + "</i>");
}
