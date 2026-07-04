//! AI Configuration Hub: multi-provider router (OpenAI-compatible,
//! Anthropic, Gemini) with local endpoints (Ollama / LM Studio).
//! All calls happen in Rust — no CORS, keys never touch the webview DOM
//! except when the user edits settings.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use crate::vault::{save_ai_result, VaultState};

#[derive(Serialize, Deserialize, Clone)]
pub struct ProviderCfg {
    pub kind: String, // "openai" | "anthropic" | "gemini"
    pub label: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AiConfig {
    pub active: String,
    pub providers: BTreeMap<String, ProviderCfg>,
}

pub struct AiState(pub Mutex<AiConfig>);

fn p(kind: &str, label: &str, model: &str, base: &str) -> ProviderCfg {
    ProviderCfg {
        kind: kind.into(),
        label: label.into(),
        api_key: String::new(),
        model: model.into(),
        base_url: base.into(),
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        providers.insert("openai".into(), p("openai", "OpenAI", "gpt-4o-mini", "https://api.openai.com/v1"));
        providers.insert("anthropic".into(), p("anthropic", "Anthropic", "claude-sonnet-5", "https://api.anthropic.com"));
        providers.insert("gemini".into(), p("gemini", "Google Gemini", "gemini-2.0-flash", "https://generativelanguage.googleapis.com/v1beta"));
        providers.insert("deepseek".into(), p("openai", "DeepSeek", "deepseek-chat", "https://api.deepseek.com/v1"));
        providers.insert("mimo".into(), p("openai", "Xiaomi MiMo", "mimo-v2-flash", "https://api.mimo.xiaomi.com/v1"));
        providers.insert("ollama".into(), p("openai", "Ollama (local)", "gemma3:1b", "http://127.0.0.1:11434/v1"));
        providers.insert("lmstudio".into(), p("openai", "LM Studio (local)", "local-model", "http://127.0.0.1:1234/v1"));
        AiConfig { active: "ollama".into(), providers }
    }
}

fn config_path() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("aura-pulse");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ai.json")
}

pub fn load_config() -> AiConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn persist(cfg: &AiConfig) {
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let path = config_path();
        let _ = std::fs::write(&path, s);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[tauri::command]
pub fn ai_get_config(state: tauri::State<'_, AiState>) -> AiConfig {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
pub fn ai_set_config(state: tauri::State<'_, AiState>, cfg: AiConfig) -> Result<(), String> {
    persist(&cfg);
    *state.0.lock().map_err(|e| e.to_string())? = cfg;
    Ok(())
}

// ---------- provider calls ----------

async fn call_provider_with_retry(
    cfg: &ProviderCfg,
    system: &str,
    user_text: &str,
    image_b64: Option<&str>,
    timeout_secs: u64,
    max_retries: u32,
) -> Result<String, String> {
    let mut last_err = String::new();
    for attempt in 0..max_retries {
        match call_provider(cfg, system, user_text, image_b64, timeout_secs).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_err = e.clone();
                // Don't retry on client errors (4xx) except 429 (rate limit)
                if e.contains("HTTP 4") && !e.contains("429") {
                    return Err(e);
                }
                // Exponential backoff before retry; must be async — a blocking
                // sleep here would stall a tokio worker thread.
                if attempt < max_retries - 1 {
                    let delay = std::time::Duration::from_millis(200 * (1 << attempt));
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(last_err)
}

async fn call_provider(
    cfg: &ProviderCfg,
    system: &str,
    user_text: &str,
    image_b64: Option<&str>,
    timeout_secs: u64,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;
    let base = cfg.base_url.trim_end_matches('/');

    // Detect local providers for better error messages
    let is_local = base.contains("127.0.0.1") || base.contains("localhost") || base.contains("lmstudio") || base.contains("ollama");

    match cfg.kind.as_str() {
        "anthropic" => {
            let mut content = vec![json!({"type": "text", "text": user_text})];
            if let Some(img) = image_b64 {
                content.insert(0, json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/png", "data": img}
                }));
            }
            let body = json!({
                "model": cfg.model,
                "max_tokens": 1500,
                "system": system,
                "messages": [{"role": "user", "content": content}]
            });
            let resp = client
                .post(format!("{}/v1/messages", base))
                .header("User-Agent", "Aura-Pulse/0.3.0")
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Anthropic API request failed: {}", e))?;
            let status = resp.status();
            let v: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("HTTP {}: {}", status, snippet(&v)));
            }
            v["content"][0]["text"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("unexpected response: {}", snippet(&v)))
        }
        "gemini" => {
            let mut parts = vec![json!({"text": user_text})];
            if let Some(img) = image_b64 {
                parts.insert(0, json!({"inline_data": {"mime_type": "image/png", "data": img}}));
            }
            let body = json!({
                "contents": [{"parts": parts}],
                "systemInstruction": {"parts": [{"text": system}]}
            });
            let url = format!("{}/models/{}:generateContent?key={}", base, cfg.model, cfg.api_key);
            let resp = client
                .post(url)
                .header("User-Agent", "Aura-Pulse/0.3.0")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Gemini API request failed: {}", e))?;
            let status = resp.status();
            let v: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("HTTP {}: {}", status, snippet(&v)));
            }
            let texts: Vec<String> = v["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|part| part["text"].as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if texts.is_empty() {
                Err(format!("unexpected response: {}", snippet(&v)))
            } else {
                Ok(texts.join(""))
            }
        }
        // default: OpenAI-compatible (OpenAI, DeepSeek, MiMo, Ollama, LM Studio)
        _ => {
            let user_content: Value = if let Some(img) = image_b64 {
                json!([
                    {"type": "text", "text": user_text},
                    {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", img)}}
                ])
            } else {
                json!(user_text)
            };
            let body = json!({
                "model": cfg.model,
                "max_tokens": 1500,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": user_content}
                ]
            });
            let mut req = client
                .post(format!("{}/chat/completions", base))
                .header("User-Agent", "Aura-Pulse/0.3.0")
                .json(&body);
            if !cfg.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", cfg.api_key));
            }
            let resp = req.send().await.map_err(|e| {
                if is_local {
                    format!("Local endpoint not reachable — ensure the server is running at {}. Details: {}", base, e)
                } else {
                    e.to_string()
                }
            })?;
            let status = resp.status();
            let v: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                let err = format!("HTTP {}: {}", status, snippet(&v));
                if is_local && status.as_u16() == 404 {
                    return Err(format!("Local endpoint not found at {} — check the URL path (should end with /v1 for OpenAI-compatible)", base));
                }
                return Err(err);
            }
            v["choices"][0]["message"]["content"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| format!("unexpected response: {}", snippet(&v)))
        }
    }
}

fn snippet(v: &Value) -> String {
    let s = v.to_string();
    s.chars().take(400).collect()
}

fn get_provider(state: &tauri::State<'_, AiState>, name: Option<&str>) -> Result<ProviderCfg, String> {
    let cfg = state.0.lock().map_err(|e| e.to_string())?;
    let key = name.map(|s| s.to_string()).unwrap_or_else(|| cfg.active.clone());
    cfg.providers
        .get(&key)
        .cloned()
        .ok_or_else(|| format!("unknown provider '{}'", key))
}

#[derive(Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
}

#[tauri::command]
pub async fn ai_test(state: tauri::State<'_, AiState>, provider: String) -> Result<TestResult, String> {
    let cfg = get_provider(&state, Some(&provider))?;
    let t0 = Instant::now();
    // Link test should fail fast, not hang for the full generation timeout.
    match call_provider_with_retry(&cfg, "You are a link tester.", "Reply with exactly: AURA LINK OK", None, 15, 2).await {
        Ok(reply) => Ok(TestResult {
            ok: true,
            latency_ms: t0.elapsed().as_millis() as u64,
            message: reply.chars().take(120).collect(),
        }),
        Err(e) => Ok(TestResult {
            ok: false,
            latency_ms: t0.elapsed().as_millis() as u64,
            message: e,
        }),
    }
}

// ---------- clip enrichment tasks ----------

struct TaskSpec {
    system: &'static str,
    prompt: &'static str,
    field: &'static str,
    needs_image: bool,
}

fn task_spec(task: &str) -> Option<TaskSpec> {
    Some(match task {
        "summarize" => TaskSpec {
            system: "You are a precise clipboard analyst. Respond ONLY with minified JSON: {\"title\": \"<max 6 words>\", \"summary\": \"<max 14 words>\", \"tags\": \"tag1,tag2,tag3\"}",
            prompt: "Analyze this clip and produce title, summary and up to 4 lowercase tags.",
            field: "summary",
            needs_image: false,
        },
        "ocr" => TaskSpec {
            system: "You are an OCR engine. Extract ALL visible text verbatim, preserving line breaks. Output only the extracted text, nothing else. If no text is present, output: (no text detected)",
            prompt: "Extract all text from this image.",
            field: "ocr",
            needs_image: true,
        },
        "describe" => TaskSpec {
            system: "You are a visual analyst. Describe the image: dominant colors (with hex estimates), shapes, layout, environment, notable objects and mood. Be structured and concise (max 180 words).",
            prompt: "Describe this image in detail.",
            field: "description",
            needs_image: true,
        },
        "markdown" => TaskSpec {
            system: "You convert content into clean, well-structured Markdown. Output ONLY the markdown document, no preamble.",
            prompt: "Convert this clip into a clean markdown document.",
            field: "markdown",
            needs_image: false,
        },
        "design" => TaskSpec {
            system: "You convert visual/textual content into a design-tokens JSON file with keys: palette (array of {name, hex}), typography, spacing, components (array of {name, description, props}). Output ONLY valid JSON.",
            prompt: "Convert this clip into a design configuration JSON.",
            field: "design_json",
            needs_image: false,
        },
        _ => return None,
    })
}

#[tauri::command]
pub async fn ai_run(
    ai: tauri::State<'_, AiState>,
    vault: tauri::State<'_, VaultState>,
    clip_id: i64,
    task: String,
) -> Result<String, String> {
    let spec = task_spec(&task).ok_or("unknown task")?;
    let cfg = get_provider(&ai, None)?;

    // Pull clip data out before any await (mutex guard is not Send).
    let (kind, content, image_b64) = {
        let conn = vault.0.lock().map_err(|e| e.to_string())?;
        let (kind, content, payload): (String, Option<String>, Option<Vec<u8>>) = conn
            .query_row(
                "SELECT kind, content, payload FROM clips WHERE id = ?1",
                rusqlite::params![clip_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        let img = payload.map(|b| {
            use base64::{engine::general_purpose::STANDARD as B64, Engine};
            B64.encode(b)
        });
        (kind, content, img)
    };

    if spec.needs_image && image_b64.is_none() {
        return Err("this task needs an image clip".into());
    }

    let mut user = spec.prompt.to_string();
    if let Some(text) = &content {
        let truncated: String = text.chars().take(14_000).collect();
        user = format!("{}\n\n--- CLIP CONTENT ({}) ---\n{}", user, kind, truncated);
    }

    let img_ref = if kind == "image" { image_b64.as_deref() } else { None };
    let reply = call_provider_with_retry(&cfg, spec.system, &user, img_ref, 120, 3).await?;

    // Persist results.
    {
        let conn = vault.0.lock().map_err(|e| e.to_string())?;
        if task == "summarize" {
            // Try to parse {title, summary, tags}; fall back to raw text.
            let cleaned = reply.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
            if let Ok(v) = serde_json::from_str::<Value>(cleaned) {
                if let Some(t) = v["title"].as_str() {
                    save_ai_result(&conn, clip_id, "title", t)?;
                }
                if let Some(s) = v["summary"].as_str() {
                    save_ai_result(&conn, clip_id, "summary", s)?;
                }
                if let Some(t) = v["tags"].as_str() {
                    save_ai_result(&conn, clip_id, "tags", t)?;
                }
            } else {
                save_ai_result(&conn, clip_id, "summary", &reply)?;
            }
        } else {
            save_ai_result(&conn, clip_id, spec.field, &reply)?;
        }
    }
    Ok(reply)
}

// ---------- AI optimization modules ----------

fn read_trim(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Snapshot of the live system state that the model tailors modules to.
fn gather_system_state() -> String {
    let mut s = String::new();
    if let Some(load) = read_trim("/proc/loadavg") {
        s += &format!("loadavg: {}\n", load);
    }
    if let Ok(ci) = std::fs::read_to_string("/proc/cpuinfo") {
        let model = ci
            .lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split(':').nth(1))
            .unwrap_or("?")
            .trim()
            .to_string();
        let count = ci.lines().filter(|l| l.starts_with("processor")).count();
        s += &format!("cpu: {} threads, {}\n", count, model);
    }
    if let Some(gov) = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor") {
        s += &format!("cpu governor: {}\n", gov);
    }
    if let Ok(mi) = std::fs::read_to_string("/proc/meminfo") {
        for key in ["MemTotal", "MemAvailable", "SwapTotal", "SwapFree"] {
            if let Some(line) = mi.lines().find(|l| l.starts_with(key)) {
                s += &format!("{}\n", line.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    if let Some(sw) = read_trim("/proc/sys/vm/swappiness") {
        s += &format!("vm.swappiness: {}\n", sw);
    }
    if let Some(up) = read_trim("/proc/uptime").and_then(|u| u.split('.').next().map(String::from)) {
        s += &format!("uptime_seconds: {}\n", up);
    }
    #[cfg(unix)]
    unsafe {
        let path = std::ffi::CString::new("/").unwrap();
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut st) == 0 {
            let total = st.f_blocks as u64 * st.f_frsize as u64;
            let free = st.f_bavail as u64 * st.f_frsize as u64;
            s += &format!("disk /: {:.0} GB total, {:.0} GB free\n", total as f64 / 1e9, free as f64 / 1e9);
        }
    }
    s
}

/// Hard safety filter for model-generated commands. Modules containing any
/// of these are dropped at generation time and refused at apply time.
fn command_blocked(cmd: &str) -> Option<&'static str> {
    const BLOCKED: [&str; 22] = [
        "rm ", "rm\t", "rmdir", "mkfs", "dd ", "shred", "> /dev/", "of=/dev/",
        "shutdown", "reboot", "poweroff", "halt", "init 0", "init 6",
        "userdel", "passwd", "chown", "chmod -R", ":(){", "swapoff",
        "curl ", "wget ",
    ];
    let lower = cmd.to_lowercase();
    BLOCKED.iter().find(|p| lower.contains(*p)).copied()
}

/// Ask the configured provider for optimization modules tailored to the
/// current system state. Generation only — nothing is executed here.
#[tauri::command]
pub async fn ai_optimize_generate(ai: tauri::State<'_, AiState>) -> Result<Value, String> {
    if cfg!(windows) {
        return Err("AI optimization modules are Linux-only for now".into());
    }
    let cfg = get_provider(&ai, None)?;
    let state = gather_system_state();
    let system = "You are a Linux performance engineer generating optimization modules for a desktop system. \
Respond ONLY with a minified JSON array, no markdown fences, of 2 to 4 objects: \
[{\"title\":\"<max 6 words>\",\"rationale\":\"<why this helps THIS system, max 30 words>\",\"impact\":\"low|medium|high\",\"risk\":\"safe|caution\",\"requires_root\":true|false,\"commands\":[\"<shell command>\"]}] \
Command rules: non-interactive, standard Linux tools (sysctl, sysfs writes via tee, powerprofilesctl, renice). \
If you use ionice: best-effort is 'ionice -c2 -n<0-7> -p <pid>', idle is 'ionice -c3 -p <pid>' — class 3 never takes -n. \
Kernel tunables (vm.*, kernel.*, net.*) live under /proc/sys — always use 'sysctl -w name=value'. Never invent /sys paths; only use sysfs files that exist on standard kernels. \
Each array element must be ONE complete standalone command — never split a loop or pipeline across elements. \
NEVER prefix commands with sudo or doas; set requires_root: true instead and the app will elevate. \
Never suggest deleting files, installing/removing packages, downloading anything, or touching users/permissions/partitions.";
    let user = format!(
        "Current system state:\n{}\nPropose optimization modules tailored to this exact state. Reference the actual numbers in your rationales.",
        state
    );
    let reply = call_provider_with_retry(&cfg, system, &user, None, 120, 2).await?;
    let cleaned = reply
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: Value = serde_json::from_str(cleaned)
        .map_err(|_| format!("model returned invalid JSON: {}", cleaned.chars().take(200).collect::<String>()))?;
    let arr = parsed.as_array().ok_or("model did not return a JSON array")?;

    let mut modules = Vec::new();
    let mut filtered = 0usize;
    for m in arr {
        let raw: Vec<String> = m["commands"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let mut requires_root = m["requires_root"].as_bool().unwrap_or(true);
        let commands = sanitize_commands(&raw, &mut requires_root);
        if commands.is_empty() {
            continue;
        }
        if commands.iter().any(|c| command_blocked(c).is_some()) {
            filtered += 1;
            continue;
        }
        // Models routinely produce scripts that don't parse — check before offering.
        if !script_parses(&join_script(&commands)) {
            filtered += 1;
            continue;
        }
        // …and hallucinate kernel paths — verify write targets exist here.
        if commands.iter().any(|c| missing_write_target(c).is_some()) {
            filtered += 1;
            continue;
        }
        // …and fumble tool argument grammar.
        if commands.iter().any(|c| invalid_tool_usage(c).is_some()) {
            filtered += 1;
            continue;
        }
        modules.push(json!({
            "title": m["title"].as_str().unwrap_or("Untitled module"),
            "rationale": m["rationale"].as_str().unwrap_or(""),
            "impact": m["impact"].as_str().unwrap_or("low"),
            "risk": m["risk"].as_str().unwrap_or("caution"),
            "requires_root": requires_root,
            "commands": commands,
        }));
    }
    if modules.is_empty() {
        return Err("the model produced no usable modules (all removed by safety rules) — try regenerating".into());
    }
    Ok(json!({ "state": state, "modules": modules, "filtered": filtered }))
}

/// Strip sudo/doas prefixes (the app elevates via pkexec instead) and
/// upgrade requires_root when a command obviously needs it.
fn sanitize_commands(raw: &[String], requires_root: &mut bool) -> Vec<String> {
    const ROOT_HINTS: [&str; 7] = [
        "sysctl -w", "sysctl --write", "tee /proc/", "tee /sys/", "/proc/sys/", "cpupower", "renice -n -",
    ];
    let mut out = Vec::new();
    for c in raw {
        let mut c = c.trim().to_string();
        if c.is_empty() {
            continue;
        }
        loop {
            if let Some(rest) = c.strip_prefix("sudo ").or_else(|| c.strip_prefix("doas ")) {
                c = rest.trim().to_string();
                *requires_root = true;
            } else {
                break;
            }
        }
        // Common model slip: 'powerprofilesctl <profile>' missing the 'set'
        // subcommand — repair it (the user reviews the repaired command).
        if let Some(rest) = c.strip_prefix("powerprofilesctl ") {
            let t = rest.trim();
            if ["performance", "balanced", "power-saver"].contains(&t) {
                c = format!("powerprofilesctl set {}", t);
            }
        }
        out.push(c);
    }
    if out.iter().any(|c| ROOT_HINTS.iter().any(|h| c.contains(h))) {
        *requires_root = true;
    }
    out
}

/// Shape-check invocations of the tools the prompt suggests — models get
/// their argument grammar wrong (missing subcommands, invalid flag combos).
fn invalid_tool_usage(cmd: &str) -> Option<String> {
    let toks: Vec<&str> = cmd.split_whitespace().collect();
    let bin = toks.first()?.rsplit('/').next().unwrap_or("");
    match bin {
        "powerprofilesctl" => {
            const SUB: [&str; 10] = [
                "list", "list-holds", "list-actions", "get", "set",
                "configure-action", "configure-battery-aware", "query-battery-aware", "launch", "version",
            ];
            match toks.get(1) {
                Some(s) if SUB.contains(s) => {
                    if *s == "set" && !matches!(toks.get(2), Some(p) if ["performance", "balanced", "power-saver"].contains(p)) {
                        return Some("powerprofilesctl set needs performance|balanced|power-saver".into());
                    }
                    None
                }
                _ => Some("powerprofilesctl needs a subcommand, e.g. 'powerprofilesctl set balanced'".into()),
            }
        }
        "ionice" => {
            let idle = cmd.contains("-c3") || cmd.contains("-c 3") || cmd.contains("--class 3");
            let has_level = toks.iter().any(|t| *t == "-n" || (t.starts_with("-n") && t[2..].chars().all(|ch| ch.is_ascii_digit()) && t.len() > 2));
            if idle && has_level {
                return Some("ionice idle class (-c3) takes no -n level".into());
            }
            None
        }
        _ => None,
    }
}

/// Models invent /sys and /proc paths. Extract write targets (redirections,
/// tee destinations, sysctl keys) and report the first one that doesn't
/// exist on THIS system — such files cannot be created, only written.
fn missing_write_target(cmd: &str) -> Option<String> {
    let exists = |p: &str| std::path::Path::new(p).exists();
    let toks: Vec<&str> = cmd.split_whitespace().collect();
    let mut sysctl_mode = false;
    for (i, t) in toks.iter().enumerate() {
        let t = *t;
        if t == "sysctl" {
            sysctl_mode = true;
            continue;
        }
        if sysctl_mode && !t.starts_with('-') && t.contains('=') {
            let key = t.split('=').next().unwrap_or("");
            if key.contains('.') {
                let path = format!("/proc/sys/{}", key.replace('.', "/"));
                if !exists(&path) {
                    return Some(format!("sysctl key '{}' ({})", key, path));
                }
            }
            continue;
        }
        // redirection: "> /path", ">> /path", ">/path", ">>/path"
        let target = if t == ">" || t == ">>" {
            toks.get(i + 1).copied()
        } else {
            t.strip_prefix(">>").or_else(|| t.strip_prefix('>'))
        };
        if let Some(p) = target {
            let p = p.trim_matches('"').trim_matches('\'');
            if (p.starts_with("/proc/") || p.starts_with("/sys/")) && !exists(p) {
                return Some(p.to_string());
            }
        }
        // tee destination (first non-flag argument)
        if t == "tee" {
            for n in &toks[i + 1..] {
                if n.starts_with('-') {
                    continue;
                }
                let p = n.trim_matches('"').trim_matches('\'');
                if (p.starts_with("/proc/") || p.starts_with("/sys/")) && !exists(p) {
                    return Some(p.to_string());
                }
                break;
            }
        }
    }
    None
}

/// Newline-joined with fail-fast — keeps multi-line shell constructs valid
/// (joining with && breaks any loop the model wrote across lines).
fn join_script(commands: &[String]) -> String {
    format!("set -e\n{}", commands.join("\n"))
}

#[cfg(test)]
mod opt_tests {
    use super::*;

    #[test]
    fn rejects_hallucinated_kernel_paths() {
        assert!(missing_write_target("echo 10 > /sys/module/misc/parameters/swappiness").is_some());
        assert!(missing_write_target("echo 10 | tee /sys/module/misc/parameters/swappiness").is_some());
        assert!(missing_write_target("sysctl -w vm.nonexistent_tunable_xyz=1").is_some());
    }

    #[test]
    fn accepts_real_kernel_paths() {
        assert!(missing_write_target("sysctl -w vm.swappiness=10").is_none());
        assert!(missing_write_target("echo 10 > /proc/sys/vm/swappiness").is_none());
        assert!(missing_write_target("echo 1 | tee /proc/sys/kernel/sched_autogroup_enabled").is_none());
        assert!(missing_write_target("renice -n 5 -p 1234").is_none());
    }

    #[test]
    fn strips_sudo_and_upgrades_root() {
        let mut root = false;
        let cmds = sanitize_commands(&["sudo sysctl -w vm.swappiness=10".to_string()], &mut root);
        assert_eq!(cmds, vec!["sysctl -w vm.swappiness=10".to_string()]);
        assert!(root);
    }

    #[test]
    fn repairs_and_validates_tool_usage() {
        let mut root = false;
        let cmds = sanitize_commands(&["powerprofilesctl balanced".to_string()], &mut root);
        assert_eq!(cmds, vec!["powerprofilesctl set balanced".to_string()]);
        assert!(invalid_tool_usage("powerprofilesctl set balanced").is_none());
        assert!(invalid_tool_usage("powerprofilesctl bogus").is_some());
        assert!(invalid_tool_usage("powerprofilesctl set turbo").is_some());
        assert!(invalid_tool_usage("ionice -c3 -n7 -p 1234").is_some());
        assert!(invalid_tool_usage("ionice -c2 -n7 -p 1234").is_none());
        assert!(invalid_tool_usage("sysctl -w vm.swappiness=10").is_none());
    }
}

/// Parse-only check (sh -n): catches malformed model output before it runs.
fn script_parses(script: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-n", "-c", script])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a reviewed module. The user has seen the exact commands; root
/// modules go through pkexec (system auth dialog), like the rest of sysopt.
#[tauri::command]
pub async fn ai_optimize_apply(commands: Vec<String>, requires_root: bool) -> Result<String, String> {
    if cfg!(windows) {
        return Err("AI optimization modules are Linux-only for now".into());
    }
    let mut requires_root = requires_root;
    let commands = sanitize_commands(&commands, &mut requires_root);
    if commands.is_empty() {
        return Err("module has no commands".into());
    }
    for c in &commands {
        if let Some(pat) = command_blocked(c) {
            return Err(format!("blocked by safety rules ('{}'): {}", pat.trim(), c));
        }
    }
    let script = join_script(&commands);
    if !script_parses(&script) {
        return Err("the model produced a malformed script (shell syntax error) — regenerate the modules".into());
    }
    for c in &commands {
        if let Some(missing) = missing_write_target(c) {
            return Err(format!("references a kernel path that doesn't exist on this system: {} — regenerate", missing));
        }
        if let Some(why) = invalid_tool_usage(c) {
            return Err(format!("invalid tool usage ({}) — regenerate", why));
        }
    }
    let out = tauri::async_runtime::spawn_blocking(move || {
        if requires_root {
            std::process::Command::new("pkexec").args(["sh", "-c", &script]).output()
        } else {
            std::process::Command::new("sh").args(["-c", &script]).output()
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("failed to run: {}", e))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(format!(
            "exit {}: {}",
            out.status.code().unwrap_or(-1),
            detail.chars().take(400).collect::<String>()
        ));
    }
    let text = stdout.trim();
    Ok(if text.is_empty() { "OK — applied".to_string() } else { text.chars().take(1200).collect() })
}

// ---------- audio transcription ----------

/// Locate a usable local whisper.cpp install (binary + ggml model) in ~/whisper.cpp.
fn find_whisper() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let root = dirs::home_dir()?.join("whisper.cpp");
    let bin = [
        "build/bin/whisper-cli", "build/bin/main", "main", "whisper-cli",
        "build/bin/Release/whisper-cli.exe", "build/bin/whisper-cli.exe", "whisper-cli.exe", "main.exe",
    ]
    .iter()
    .map(|b| root.join(b))
    .find(|p| p.is_file())?;
    let model = std::fs::read_dir(root.join("models"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("ggml-") && name.ends_with(".bin") && !name.contains("encoder")
        })?;
    Some((bin, model))
}

fn run_whisper(bin: &std::path::Path, model: &std::path::Path, audio: &[u8], ext: &str) -> Result<String, String> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let input = dir.join(format!("aura-audio-{}.{}", stamp, ext));
    let wav = dir.join(format!("aura-audio-{}.wav", stamp));
    std::fs::write(&input, audio).map_err(|e| e.to_string())?;

    // whisper.cpp wants 16 kHz mono WAV; convert unless it already is one.
    let feed = if ext == "wav" {
        input.clone()
    } else {
        let ff = std::process::Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(&input)
            .args(["-ar", "16000", "-ac", "1"])
            .arg(&wav)
            .output()
            .map_err(|e| format!("ffmpeg not available: {}", e))?;
        if !ff.status.success() {
            let _ = std::fs::remove_file(&input);
            return Err(format!("ffmpeg failed: {}", String::from_utf8_lossy(&ff.stderr).chars().take(200).collect::<String>()));
        }
        wav.clone()
    };

    let out = std::process::Command::new(bin)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(&feed)
        .args(["-nt", "-np"])
        .output();
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&wav);
    let out = out.map_err(|e| format!("whisper.cpp failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!("whisper.cpp failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err("whisper.cpp produced no output".into());
    }
    Ok(text)
}

async fn gemini_transcribe(cfg: &ProviderCfg, mime: &str, data_b64: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {}", e))?;
    let base = cfg.base_url.trim_end_matches('/');
    let body = json!({
        "contents": [{"parts": [
            {"inline_data": {"mime_type": mime, "data": data_b64}},
            {"text": prompt}
        ]}]
    });
    let url = format!("{}/models/{}:generateContent?key={}", base, cfg.model, cfg.api_key);
    let resp = client
        .post(url)
        .header("User-Agent", "Aura-Pulse/0.3.0")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini API request failed: {}", e))?;
    let status = resp.status();
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, snippet(&v)));
    }
    v["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("unexpected Gemini response: {}", snippet(&v)))
}

/// Transcribe or describe an audio clip. mode "transcribe" (default) tries
/// local whisper.cpp first, then a Gemini provider; the transcript lands in
/// the ocr column. mode "describe" is Gemini-only and fills description.
#[tauri::command]
pub async fn ai_transcribe(
    ai: tauri::State<'_, AiState>,
    vault: tauri::State<'_, VaultState>,
    clip_id: i64,
    mode: Option<String>,
) -> Result<String, String> {
    let describe = mode.as_deref() == Some("describe");
    let (kind, content, payload) = {
        let conn = vault.0.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT kind, content, payload FROM clips WHERE id = ?1",
            rusqlite::params![clip_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<Vec<u8>>>(2)?)),
        )
        .map_err(|e| e.to_string())?
    };
    if kind != "audio" {
        return Err("this task needs an audio clip".into());
    }
    let bytes = payload.ok_or("audio clip has no stored data")?;

    // File extension survives inside the "[audio] name (size)" label.
    let ext = content
        .as_deref()
        .and_then(|c| c.rsplit_once('.'))
        .map(|(_, rest)| rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "mp3".into());

    // whisper.cpp can only transcribe — description always goes to Gemini.
    let mut whisper_err = String::new();
    if !describe {
        if let Some((bin, model)) = find_whisper() {
            let b = bytes.clone();
            let e = ext.clone();
            match tauri::async_runtime::spawn_blocking(move || run_whisper(&bin, &model, &b, &e))
                .await
                .map_err(|e| e.to_string())?
            {
                Ok(text) => {
                    let conn = vault.0.lock().map_err(|e| e.to_string())?;
                    save_ai_result(&conn, clip_id, "ocr", &text)?;
                    return Ok(text);
                }
                Err(e) => whisper_err = format!(" (whisper.cpp: {})", e),
            }
        }
    }

    // Gemini fallback: the active provider if it's Gemini, else the configured one.
    let gcfg = match get_provider(&ai, None) {
        Ok(c) if c.kind == "gemini" && !c.api_key.is_empty() => Some(c),
        _ => get_provider(&ai, Some("gemini")).ok().filter(|c| !c.api_key.is_empty()),
    };
    let Some(gcfg) = gcfg else {
        return Err(if describe {
            "Audio description needs a Gemini API key — add one in the AI hub".into()
        } else {
            format!(
                "No audio-capable AI available — add a Gemini API key in the AI hub, or build whisper.cpp in ~/whisper.cpp{}",
                whisper_err
            )
        });
    };
    let mime = match ext.as_str() {
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        _ => "audio/mpeg",
    };
    let data_b64 = {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        B64.encode(&bytes)
    };
    let prompt = if describe {
        "Describe this audio in detail: what kind of audio it is (speech, music, ambience, effects), voices and their tone, instruments, mood, notable sounds and structure. Be concise (max 150 words)."
    } else {
        "Transcribe this audio verbatim. If there is no speech, describe the audio instead (music, sounds, ambience). Output only the transcript or description."
    };
    let text = gemini_transcribe(&gcfg, mime, &data_b64, prompt).await?;
    let conn = vault.0.lock().map_err(|e| e.to_string())?;
    save_ai_result(&conn, clip_id, if describe { "description" } else { "ocr" }, &text)?;
    Ok(text)
}

#[tauri::command]
pub async fn ai_chat(
    ai: tauri::State<'_, AiState>,
    vault: tauri::State<'_, VaultState>,
    prompt: String,
    clip_id: Option<i64>,
) -> Result<String, String> {
    let cfg = get_provider(&ai, None)?;
    let mut user = prompt.clone();
    let mut image_b64: Option<String> = None;

    if let Some(id) = clip_id {
        let conn = vault.0.lock().map_err(|e| e.to_string())?;
        if let Ok((kind, content, payload)) = conn.query_row(
            "SELECT kind, content, payload FROM clips WHERE id = ?1",
            rusqlite::params![id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<Vec<u8>>>(2)?)),
        ) {
            if let Some(text) = content {
                let truncated: String = text.chars().take(14_000).collect();
                user = format!("{}\n\n--- ATTACHED CLIP ({}) ---\n{}", prompt, kind, truncated);
            }
            if kind == "image" {
                use base64::{engine::general_purpose::STANDARD as B64, Engine};
                image_b64 = payload.map(|b| B64.encode(b));
            }
        }
    }

    call_provider_with_retry(
        &cfg,
        "You are Aura, a sharp assistant embedded in a system-monitor/clipboard app. Be concise and useful.",
        &user,
        image_b64.as_deref(),
        120,
        3,
    )
    .await
}
