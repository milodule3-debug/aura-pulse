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
        .map_err(|e| e.to_string())?;
    let base = cfg.base_url.trim_end_matches('/');

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
                .header("x-api-key", &cfg.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
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
            let resp = client.post(url).json(&body).send().await.map_err(|e| e.to_string())?;
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
            let mut req = client.post(format!("{}/chat/completions", base)).json(&body);
            if !cfg.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", cfg.api_key));
            }
            let resp = req.send().await.map_err(|e| e.to_string())?;
            let status = resp.status();
            let v: Value = resp.json().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("HTTP {}: {}", status, snippet(&v)));
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
    match call_provider(&cfg, "You are a link tester.", "Reply with exactly: AURA LINK OK", None, 15).await {
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
    let reply = call_provider(&cfg, spec.system, &user, img_ref, 120).await?;

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

    call_provider(
        &cfg,
        "You are Aura, a sharp assistant embedded in a system-monitor/clipboard app. Be concise and useful.",
        &user,
        image_b64.as_deref(),
        120,
    )
    .await
}
