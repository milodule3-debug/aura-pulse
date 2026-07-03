//! Hardware benchmark suite with LLM-performance estimation.
//! Compute work runs on blocking threads; progress streams to the UI
//! via `bench_progress` events.

use serde::Serialize;
use serde_json::{json, Value};
use std::hint::black_box;
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
struct Progress {
    test: String,
    pct: f32,
    label: String,
}

fn progress(app: &AppHandle, test: &str, pct: f32, label: &str) {
    let _ = app.emit(
        "bench_progress",
        Progress { test: test.into(), pct, label: label.into() },
    );
}

// ---------- CPU ----------

#[inline(always)]
fn mix(mut x: u64, iters: u64) -> u64 {
    for _ in 0..iters {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    }
    x
}

/// Run the mixer for ~`secs` seconds, returning million-iterations/sec.
fn cpu_burn(secs: f64) -> f64 {
    let chunk: u64 = 2_000_000;
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut done: u64 = 0;
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f64() < secs {
        x = black_box(mix(x, chunk));
        done += chunk;
    }
    black_box(x);
    done as f64 / t0.elapsed().as_secs_f64() / 1e6
}

fn bench_cpu(app: &AppHandle) -> Value {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    progress(app, "cpu", 5.0, "single-core burn");
    let single = cpu_burn(1.2);
    progress(app, "cpu", 45.0, &format!("multi-core burn ({} threads)", threads));
    let handles: Vec<_> = (0..threads)
        .map(|_| std::thread::spawn(|| cpu_burn(1.2)))
        .collect();
    let multi: f64 = handles.into_iter().map(|h| h.join().unwrap_or(0.0)).sum();
    progress(app, "cpu", 100.0, "done");
    json!({
        "single_mops": single,
        "multi_mops": multi,
        "threads": threads,
        "scaling": if single > 0.0 { multi / single } else { 0.0 },
    })
}

// ---------- memory ----------

fn bench_memory(app: &AppHandle) -> Value {
    const N: usize = 16 * 1024 * 1024; // 128 MB per buffer
    let bytes = (N * 8) as f64;

    progress(app, "memory", 10.0, "allocating 256 MB");
    let mut a = vec![0u64; N];
    let mut b = vec![0u64; N];

    progress(app, "memory", 25.0, "write pass");
    let t0 = Instant::now();
    for (i, v) in a.iter_mut().enumerate() {
        *v = i as u64;
    }
    black_box(&a);
    let write_gbps = bytes / t0.elapsed().as_secs_f64() / 1e9;

    progress(app, "memory", 55.0, "read pass");
    let t0 = Instant::now();
    let mut sum = 0u64;
    for v in &a {
        sum = sum.wrapping_add(*v);
    }
    black_box(sum);
    let read_gbps = bytes / t0.elapsed().as_secs_f64() / 1e9;

    progress(app, "memory", 80.0, "copy pass");
    let t0 = Instant::now();
    b.copy_from_slice(&a);
    black_box(&b);
    let copy_gbps = bytes * 2.0 / t0.elapsed().as_secs_f64() / 1e9;

    progress(app, "memory", 100.0, "done");
    json!({
        "gbps_read": read_gbps,
        "gbps_write": write_gbps,
        "gbps_copy": copy_gbps,
    })
}

// ---------- disk ----------

fn bench_disk(app: &AppHandle) -> Result<Value, String> {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("aura-pulse");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("bench.tmp");

    const BLOCK: usize = 8 * 1024 * 1024;
    const BLOCKS: usize = 24; // 192 MB
    let block: Vec<u8> = (0..BLOCK).map(|i| (i * 2654435761 >> 16) as u8).collect();

    progress(app, "disk", 10.0, "sequential write 192 MB");
    let t0 = Instant::now();
    {
        let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        for i in 0..BLOCKS {
            f.write_all(&block).map_err(|e| e.to_string())?;
            progress(app, "disk", 10.0 + 40.0 * (i as f32 / BLOCKS as f32), "writing");
        }
        f.sync_all().map_err(|e| e.to_string())?;
    }
    let write_mbps = (BLOCK * BLOCKS) as f64 / t0.elapsed().as_secs_f64() / 1e6;

    progress(app, "disk", 60.0, "sequential read");
    let t0 = Instant::now();
    {
        let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; BLOCK];
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            black_box(&buf[..n]);
        }
    }
    let read_mbps = (BLOCK * BLOCKS) as f64 / t0.elapsed().as_secs_f64() / 1e6;
    let _ = std::fs::remove_file(&path);

    progress(app, "disk", 100.0, "done");
    Ok(json!({
        "write_mbps": write_mbps,
        "read_mbps": read_mbps,
        "note": "read is OS-cache assisted",
        "path": dir.to_string_lossy(),
    }))
}

// ---------- LLM estimation ----------

async fn probe_ollama() -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .ok()?;
    let tags: Value = client
        .get("http://127.0.0.1:11434/api/tags")
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let model = tags["models"][0]["name"].as_str()?.to_string();
    let resp: Value = client
        .post("http://127.0.0.1:11434/api/generate")
        .json(&json!({
            "model": model,
            "prompt": "Write one short sentence about neon cities.",
            "stream": false,
            "options": {"num_predict": 48}
        }))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let eval_count = resp["eval_count"].as_f64()?;
    let eval_ns = resp["eval_duration"].as_f64()?;
    let ptok = resp["prompt_eval_count"].as_f64().unwrap_or(0.0);
    let pns = resp["prompt_eval_duration"].as_f64().unwrap_or(1.0);
    Some(json!({
        "model": model,
        "tok_s": eval_count / (eval_ns / 1e9),
        "prompt_tok_s": if pns > 0.0 { ptok / (pns / 1e9) } else { 0.0 },
    }))
}

async fn bench_llm(app: AppHandle) -> Value {
    progress(&app, "llm", 10.0, "measuring memory bandwidth");
    let app2 = app.clone();
    let mem = tauri::async_runtime::spawn_blocking(move || bench_memory(&app2))
        .await
        .unwrap_or_else(|_| json!({}));
    let bw = mem["gbps_copy"].as_f64().unwrap_or(10.0);

    // Token generation on CPU is memory-bound: every token streams the
    // whole quantized weight set. tok/s ≈ effective_bandwidth / model_bytes.
    let models = [
        ("1B q4", 0.75),
        ("3B q4", 2.0),
        ("7B q4", 4.4),
        ("13B q4", 7.9),
        ("34B q4", 19.5),
    ];
    let eff = bw * 0.72; // real inference reaches ~70% of streaming copy bandwidth
    let estimates: Vec<Value> = models
        .iter()
        .map(|(name, gb)| {
            json!({"model": name, "tok_s": eff / gb, "fits_ram": *gb < 10.0})
        })
        .collect();

    progress(&app, "llm", 70.0, "probing Ollama for a real run");
    let ollama = probe_ollama().await;
    progress(&app, "llm", 100.0, "done");
    json!({
        "bandwidth_gbps": bw,
        "effective_gbps": eff,
        "estimates": estimates,
        "ollama": ollama,
    })
}

// ---------- entrypoint ----------

#[tauri::command]
pub async fn bench_run(app: AppHandle, test: String) -> Result<Value, String> {
    match test.as_str() {
        "cpu" => {
            let a = app.clone();
            tauri::async_runtime::spawn_blocking(move || bench_cpu(&a))
                .await
                .map_err(|e| e.to_string())
        }
        "memory" => {
            let a = app.clone();
            tauri::async_runtime::spawn_blocking(move || bench_memory(&a))
                .await
                .map_err(|e| e.to_string())
        }
        "disk" => {
            let a = app.clone();
            tauri::async_runtime::spawn_blocking(move || bench_disk(&a))
                .await
                .map_err(|e| e.to_string())?
        }
        "llm" => Ok(bench_llm(app).await),
        _ => Err(format!("unknown test '{}'", test)),
    }
}
