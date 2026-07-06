//! The Vault: clipboard history with SQLite persistence.
//! A watcher thread polls the system clipboard (arboard, with a
//! wl-paste fallback for exotic Wayland setups) and records every
//! new text or image clip, auto-classified, capped at 5000 entries.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

pub const MAX_CLIPS: i64 = 5000;

pub struct VaultState(pub Mutex<Connection>);

pub fn db_path() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("aura-pulse");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("vault.db")
}

pub fn open_db() -> Connection {
    let conn = Connection::open(db_path()).expect("open vault db");
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clips (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            content TEXT,
            payload BLOB,
            thumb BLOB,
            width INTEGER DEFAULT 0,
            height INTEGER DEFAULT 0,
            hash TEXT NOT NULL,
            pinned INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL,
            title TEXT, summary TEXT, tags TEXT,
            ocr TEXT, description TEXT, markdown TEXT, design_json TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_clips_created ON clips(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_clips_hash ON clips(hash);",
    )
    .expect("create schema");
    // Migration v1: deduplicate rows with duplicate hashes, then add UNIQUE
    // constraint to prevent races between the watcher connection and
    // command handler connections.
    conn.execute_batch(
        "DELETE FROM clips WHERE id NOT IN (
            SELECT MIN(id) FROM clips GROUP BY hash
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_clips_hash_unique ON clips(hash);",
    )
    .ok(); // non-fatal — if migration was already applied the index exists
    conn
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn hash_of(bytes: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}:{}", h.finish(), bytes.len())
}

// ---------- classification ----------

pub fn classify(text: &str) -> &'static str {
    let t = text.trim();
    let lines: Vec<&str> = t.lines().collect();
    if lines.len() == 1 {
        let l = lines[0];
        if l.starts_with("http://") || l.starts_with("https://") || l.starts_with("www.") {
            return "url";
        }
        if l.contains('@')
            && !l.contains(' ')
            && l.split('@').count() == 2
            && l.split('@').nth(1).map(|d| d.contains('.')).unwrap_or(false)
        {
            return "email";
        }
        if (l.starts_with('#') && (l.len() == 4 || l.len() == 7 || l.len() == 9)
            && l[1..].chars().all(|c| c.is_ascii_hexdigit()))
            || l.starts_with("rgb(")
            || l.starts_with("rgba(")
            || l.starts_with("hsl(")
        {
            return "color";
        }
    }
    if (t.starts_with('{') || t.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(t).is_ok()
    {
        return "json";
    }
    if lines.len() >= 2 {
        let commas: Vec<usize> = lines.iter().take(6).map(|l| l.matches(',').count()).collect();
        if commas[0] >= 1 && commas.iter().all(|&c| c == commas[0]) {
            return "csv";
        }
    }
    let code_hits = ["fn ", "def ", "const ", "let ", "import ", "class ", "#include", "=> ", "function ", "pub ", "return ", "if ("]
        .iter()
        .filter(|k| t.contains(**k))
        .count();
    let symbols = t.matches(|c| "{};=<>".contains(c)).count();
    if code_hits >= 1 && (symbols > 4 || lines.len() > 2) {
        return "code";
    }
    "text"
}

// ---------- clipboard watcher ----------

enum ClipData {
    Text(String),
    Image { png: Vec<u8>, w: u32, h: u32 },
    Audio { bytes: Vec<u8>, name: String },
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())?;
    let mut full = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut full, image::ImageFormat::Png)
        .ok()?;
    Some(full.into_inner())
}

fn wl_paste_text() -> Option<String> {
    let out = Command::new("wl-paste").args(["-n", "--type", "text"]).output().ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        if !s.trim().is_empty() {
            return Some(s);
        }
    }
    None
}

fn insert_clip(conn: &Connection, data: ClipData, app: &AppHandle) -> Option<i64> {
    let (kind, content, payload, thumb, w, h, hash) = match data {
        ClipData::Text(t) => {
            if t.trim().is_empty() || t.len() > 2_000_000 {
                return None;
            }
            let hash = hash_of(t.as_bytes());
            (classify(&t).to_string(), Some(t), None::<Vec<u8>>, None::<Vec<u8>>, 0u32, 0u32, hash)
        }
        ClipData::Image { png, w, h } => {
            let hash = hash_of(&png);
            let thumb = encode_png_thumb(&png);
            ("image".to_string(), None, Some(png), thumb, w, h, hash)
        }
        ClipData::Audio { bytes, name } => {
            if bytes.is_empty() {
                return None;
            }
            let hash = hash_of(&bytes);
            let label = format!("[audio] {} ({:.1} MB)", name, bytes.len() as f64 / 1e6);
            ("audio".to_string(), Some(label), Some(bytes), None, 0u32, 0u32, hash)
        }
    };

    // UPSERT: atomically insert or bump timestamp. With the UNIQUE(hash)
    // constraint this is race-safe even across SQLite connections (watcher
    // thread vs command handler). The existing SELECT-then-branch pattern
    // had a TOCTOU gap between two connections.
    let now = now_ms();
    conn.execute(
        "INSERT INTO clips (kind, content, payload, thumb, width, height, hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(hash) DO UPDATE SET created_at = excluded.created_at",
        params![kind, content, payload, thumb, w, h, hash, now],
    )
    .ok();
    let id = conn.last_insert_rowid();
    conn.execute(
        "DELETE FROM clips WHERE pinned = 0 AND id NOT IN
         (SELECT id FROM clips ORDER BY pinned DESC, created_at DESC LIMIT ?1)",
        params![MAX_CLIPS],
    )
    .ok();
    let _ = app.emit("vault_changed", 0);
    Some(id)
}

fn encode_png_thumb(png: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let thumb = img.thumbnail(360, 360);
    let mut buf = Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

pub fn spawn_watcher(app: AppHandle) {
    std::thread::Builder::new()
        .name("clipwatch".into())
        .spawn(move || watch(app))
        .expect("spawn clipboard watcher");
}

fn watch(app: AppHandle) {
    let conn = open_db();
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut last_hash = String::new();

    // Seed with newest stored hash so restarting doesn't re-capture.
    if let Ok(h) = conn.query_row(
        "SELECT hash FROM clips ORDER BY created_at DESC LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    ) {
        last_hash = h;
    }

    loop {
        let mut captured: Option<(String, ClipData)> = None;

        if let Some(cb) = clipboard.as_mut() {
            if let Ok(t) = cb.get_text() {
                if !t.trim().is_empty() {
                    let h = hash_of(t.as_bytes());
                    if h != last_hash {
                        captured = Some((h, ClipData::Text(t)));
                    }
                }
            }
            if captured.is_none() {
                if let Ok(img) = cb.get_image() {
                    let (w, hgt) = (img.width as u32, img.height as u32);
                    if w > 0 && hgt > 0 && img.bytes.len() as u64 <= 64 * 1024 * 1024 {
                        // hash raw rgba cheaply before the (expensive) png encode
                        let rh = hash_of(&img.bytes);
                        if rh != last_hash {
                            if let Some(png) = encode_png(&img.bytes, w, hgt) {
                                captured = Some((rh, ClipData::Image { png, w, h: hgt }));
                            }
                        }
                    }
                }
            }
        } else {
            // arboard failed to init (unusual Wayland compositor) — text-only fallback
            if let Some(t) = wl_paste_text() {
                let h = hash_of(t.as_bytes());
                if h != last_hash {
                    captured = Some((h, ClipData::Text(t)));
                }
            }
        }

        if let Some((h, data)) = captured {
            last_hash = h;
            insert_clip(&conn, data, &app);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

// ---------- commands ----------

#[derive(Serialize)]
pub struct ClipRow {
    pub id: i64,
    pub kind: String,
    pub content: Option<String>,
    pub thumb: Option<String>,
    pub width: u32,
    pub height: u32,
    pub pinned: bool,
    pub created_at: i64,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub tags: Option<String>,
    pub has_ai: bool,
}

#[derive(Deserialize)]
pub struct ListArgs {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub pinned_only: Option<bool>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[tauri::command]
pub fn vault_list(state: tauri::State<'_, VaultState>, args: ListArgs) -> Result<Vec<ClipRow>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut sql = String::from(
        "SELECT id, kind, content, thumb, width, height, pinned, created_at,
                title, summary, tags,
                (ocr IS NOT NULL OR description IS NOT NULL OR markdown IS NOT NULL OR design_json IS NOT NULL OR summary IS NOT NULL)
         FROM clips WHERE 1=1",
    );
    let mut params_v: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(q) = args.query.as_ref().filter(|q| !q.trim().is_empty()) {
        sql.push_str(" AND (content LIKE ?1 OR title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR ocr LIKE ?1 OR description LIKE ?1)");
        params_v.push(Box::new(format!("%{}%", q.trim())));
    }
    if let Some(k) = args.kind.as_ref().filter(|k| !k.is_empty() && *k != "all") {
        sql.push_str(&format!(" AND kind = ?{}", params_v.len() + 1));
        params_v.push(Box::new(k.clone()));
    }
    if args.pinned_only.unwrap_or(false) {
        sql.push_str(" AND pinned = 1");
    }
    sql.push_str(&format!(
        " ORDER BY pinned DESC, created_at DESC LIMIT {} OFFSET {}",
        args.limit.unwrap_or(60).min(200),
        args.offset.unwrap_or(0).max(0)
    ));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let refs: Vec<&dyn rusqlite::ToSql> = params_v.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(ClipRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                content: r.get::<_, Option<String>>(2)?.map(|c| c.chars().take(600).collect()),
                thumb: r
                    .get::<_, Option<Vec<u8>>>(3)?
                    .map(|b| format!("data:image/png;base64,{}", B64.encode(b))),
                width: r.get::<_, Option<u32>>(4)?.unwrap_or(0),
                height: r.get::<_, Option<u32>>(5)?.unwrap_or(0),
                pinned: r.get::<_, i64>(6)? != 0,
                created_at: r.get(7)?,
                title: r.get(8)?,
                summary: r.get(9)?,
                tags: r.get(10)?,
                has_ai: r.get::<_, i64>(11)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[derive(Serialize)]
pub struct ClipFull {
    pub id: i64,
    pub kind: String,
    pub content: Option<String>,
    pub image: Option<String>,
    pub width: u32,
    pub height: u32,
    pub pinned: bool,
    pub created_at: i64,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub tags: Option<String>,
    pub ocr: Option<String>,
    pub description: Option<String>,
    pub markdown: Option<String>,
    pub design_json: Option<String>,
}

#[tauri::command]
pub fn vault_get(state: tauri::State<'_, VaultState>, id: i64) -> Result<ClipFull, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "SELECT id, kind, content, payload, width, height, pinned, created_at,
                title, summary, tags, ocr, description, markdown, design_json
         FROM clips WHERE id = ?1",
        params![id],
        |r| {
            Ok(ClipFull {
                id: r.get(0)?,
                kind: r.get(1)?,
                content: r.get(2)?,
                image: r
                    .get::<_, Option<Vec<u8>>>(3)?
                    .map(|b| format!("data:image/png;base64,{}", B64.encode(b))),
                width: r.get::<_, Option<u32>>(4)?.unwrap_or(0),
                height: r.get::<_, Option<u32>>(5)?.unwrap_or(0),
                pinned: r.get::<_, i64>(6)? != 0,
                created_at: r.get(7)?,
                title: r.get(8)?,
                summary: r.get(9)?,
                tags: r.get(10)?,
                ocr: r.get(11)?,
                description: r.get(12)?,
                markdown: r.get(13)?,
                design_json: r.get(14)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn vault_delete(state: tauri::State<'_, VaultState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM clips WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn vault_pin(state: tauri::State<'_, VaultState>, id: i64, pinned: bool) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE clips SET pinned = ?1 WHERE id = ?2", params![pinned as i64, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn vault_wipe(state: tauri::State<'_, VaultState>) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM clips", [])
        .map_err(|e| e.to_string())?;
    conn.execute_batch("VACUUM;").ok();
    Ok(())
}

#[tauri::command]
pub fn vault_copy(state: tauri::State<'_, VaultState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let (kind, content, payload): (String, Option<String>, Option<Vec<u8>>) = conn
        .query_row(
            "SELECT kind, content, payload FROM clips WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if kind == "image" {
        let png = payload.ok_or("no image payload")?;
        let img = image::load_from_memory(&png).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        cb.set_image(arboard::ImageData {
            width: w,
            height: h,
            bytes: rgba.into_raw().into(),
        })
        .map_err(|e| e.to_string())?;
    } else {
        cb.set_text(content.unwrap_or_default()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn vault_add_text(state: tauri::State<'_, VaultState>, app: AppHandle, content: String) -> Result<i64, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    Ok(insert_clip(&conn, ClipData::Text(content), &app).unwrap_or(0))
}

fn store_image_bytes(conn: &Connection, bytes: &[u8], app: &AppHandle) -> Result<i64, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(insert_clip(conn, ClipData::Image { png: png.into_inner(), w, h }, app).unwrap_or(0))
}

/// Decode a payload that may be raw base64 or a data URL ("data:...;base64,<payload>").
fn b64_payload(data_b64: &str) -> Result<Vec<u8>, String> {
    let raw = match data_b64.split_once(',') {
        Some((head, tail)) if head.contains("base64") => tail,
        _ => data_b64,
    };
    let bytes = B64.decode(raw.trim()).map_err(|e| format!("invalid base64 payload: {}", e))?;
    if bytes.is_empty() {
        return Err("empty payload".into());
    }
    Ok(bytes)
}

#[tauri::command]
pub fn vault_add_image(state: tauri::State<'_, VaultState>, app: AppHandle, data_b64: String) -> Result<i64, String> {
    let bytes = b64_payload(&data_b64)?;
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    store_image_bytes(&conn, &bytes, &app)
}

#[tauri::command]
pub fn vault_add_audio(state: tauri::State<'_, VaultState>, app: AppHandle, data_b64: String, name: String) -> Result<i64, String> {
    let bytes = b64_payload(&data_b64)?;
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    Ok(insert_clip(&conn, ClipData::Audio { bytes, name }, &app).unwrap_or(0))
}

/// Ingest a file dropped via the native (Tauri) drag-drop event, which
/// delivers paths rather than file contents.
#[tauri::command]
pub fn vault_add_path(state: tauri::State<'_, VaultState>, app: AppHandle, path: String, kind: String) -> Result<i64, String> {
    const IMAGE_EXT: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff"];
    const AUDIO_EXT: [&str; 8] = ["mp3", "wav", "ogg", "flac", "m4a", "opus", "aac", "wma"];
    const MAX_BYTES: u64 = 100 * 1024 * 1024; // 100 MB
    let p = std::path::Path::new(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match kind.as_str() {
        "image" => {
            if !IMAGE_EXT.contains(&ext.as_str()) {
                return Err("Not an image file".into());
            }
            let meta = std::fs::metadata(p).map_err(|e| format!("failed to stat {}: {}", path, e))?;
            if meta.len() > MAX_BYTES {
                return Err(format!("file too large ({:.1} MB, max 100 MB)", meta.len() as f64 / 1e6));
            }
            let bytes = std::fs::read(p).map_err(|e| format!("failed to read {}: {}", path, e))?;
            let conn = state.0.lock().map_err(|e| e.to_string())?;
            store_image_bytes(&conn, &bytes, &app)
        }
        "audio" => {
            if !AUDIO_EXT.contains(&ext.as_str()) {
                return Err("Not an audio file".into());
            }
            let meta = std::fs::metadata(p).map_err(|e| format!("failed to stat {}: {}", path, e))?;
            if meta.len() > MAX_BYTES {
                return Err(format!("file too large ({:.1} MB, max 100 MB)", meta.len() as f64 / 1e6));
            }
            let bytes = std::fs::read(p).map_err(|e| format!("failed to read {}: {}", path, e))?;
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(path.as_str()).to_string();
            let conn = state.0.lock().map_err(|e| e.to_string())?;
            Ok(insert_clip(&conn, ClipData::Audio { bytes, name }, &app).unwrap_or(0))
        }
        other => Err(format!("unknown drop kind: {}", other)),
    }
}

#[derive(Serialize)]
pub struct VaultStats {
    pub total: i64,
    pub by_kind: Vec<(String, i64)>,
    pub db_bytes: u64,
    pub pinned: i64,
}

#[tauri::command]
pub fn vault_stats(state: tauri::State<'_, VaultState>) -> Result<VaultStats, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0))
        .unwrap_or(0);
    let pinned: i64 = conn
        .query_row("SELECT COUNT(*) FROM clips WHERE pinned = 1", [], |r| r.get(0))
        .unwrap_or(0);
    let mut stmt = conn
        .prepare("SELECT kind, COUNT(*) FROM clips GROUP BY kind ORDER BY 2 DESC")
        .map_err(|e| e.to_string())?;
    let by_kind = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let db_bytes = std::fs::metadata(db_path()).map(|m| m.len()).unwrap_or(0);
    Ok(VaultStats { total, by_kind, db_bytes, pinned })
}

/// Store an AI result back onto a clip column.
pub fn save_ai_result(conn: &Connection, id: i64, field: &str, value: &str) -> Result<(), String> {
    let col = match field {
        "summary" | "title" | "tags" | "ocr" | "description" | "markdown" | "design_json" => field,
        _ => return Err("bad field".into()),
    };
    conn.execute(
        &format!("UPDATE clips SET {} = ?1 WHERE id = ?2", col),
        params![value, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn safe_filename(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .take(30)
        .collect::<String>()
        .trim()
        .to_string()
}

#[tauri::command]
pub fn vault_save_as(state: tauri::State<'_, VaultState>, id: i64) -> Result<String, String> {
    // Fetch clip data first, then drop the lock before opening the file dialog
    let (kind, content, payload) = {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT kind, content, payload FROM clips WHERE id = ?1",
            params![id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<Vec<u8>>>(2)?)),
        )
        .map_err(|e| e.to_string())?
    };

    let default_name = match kind.as_str() {
        "image" => format!("vault_clip_{}.png", id),
        "audio" => format!("vault_clip_{}.mp3", id),
        "json" | "csv" => {
            let stem = content.as_deref().map(safe_filename).filter(|s| !s.is_empty());
            format!("{}.{}", stem.unwrap_or_else(|| format!("vault_clip_{}", id)), kind)
        }
        _ => {
            let stem = content.as_deref().map(safe_filename).filter(|s| !s.is_empty());
            format!("{}.txt", stem.unwrap_or_else(|| format!("vault_clip_{}", id)))
        }
    };

    let ext = match kind.as_str() {
        "image" => "png",
        "audio" => "mp3",
        "json" | "csv" => kind.as_str(),
        _ => "txt",
    };

    let dialog = rfd::FileDialog::new()
        .set_file_name(&default_name)
        .add_filter("Save as", &[ext]);

    let path = dialog.save_file().ok_or("cancelled")?;

    match kind.as_str() {
        "image" | "audio" => {
            let bytes = payload.ok_or(format!("no payload for {} clip", kind))?;
            std::fs::write(&path, bytes).map_err(|e| format!("write failed: {}", e))?;
        }
        _ => {
            let text = content.as_deref().unwrap_or("");
            let output = match kind.as_str() {
                "color" => format!("Color: {}\nHEX: {}", text, text),
                _ => text.to_string(),
            };
            std::fs::write(&path, output).map_err(|e| format!("write failed: {}", e))?;
        }
    }

    Ok(path.to_string_lossy().to_string())
}
