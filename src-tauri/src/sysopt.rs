//! System optimization: power profiles (via power-profiles-daemon,
//! no root needed), CPU boost, swappiness and cache dropping
//! (privileged writes go through pkexec → system auth dialog).
//! macOS: these features are not applicable; commands return errors.

use serde::Serialize;

#[derive(Serialize)]
pub struct SysoptInfo {
    pub profile: String,
    pub profiles: Vec<String>,
    pub governor: String,
    pub epp: Option<String>,
    pub boost: Option<bool>,
    pub swappiness: u32,
    pub has_ppd: bool,
}

// ---------- Linux implementation ----------

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
fn boost_paths() -> Vec<String> {
    let global = "/sys/devices/system/cpu/cpufreq/boost";
    if std::path::Path::new(global).exists() {
        return vec![global.to_string()];
    }
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/devices/system/cpu/cpufreq") {
        for e in rd.flatten() {
            let p = e.path().join("boost");
            if p.exists() {
                v.push(p.to_string_lossy().to_string());
            }
        }
    }
    v
}

#[cfg(target_os = "linux")]
fn read_trim(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub fn sysopt_get() -> SysoptInfo {
    let ppd = Command::new("powerprofilesctl").arg("get").output();
    let (has_ppd, profile) = match ppd {
        Ok(o) if o.status.success() => (true, String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => (false, String::new()),
    };
    let profiles = if has_ppd {
        Command::new("powerprofilesctl")
            .arg("list")
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| {
                        let t = l.trim().trim_start_matches("* ").trim();
                        if t.ends_with(':') && !t.contains(' ') {
                            Some(t.trim_end_matches(':').to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let boost = boost_paths()
        .first()
        .and_then(|p| read_trim(p))
        .map(|v| v == "1");

    SysoptInfo {
        profile,
        profiles,
        governor: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor").unwrap_or_default(),
        epp: read_trim("/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference"),
        boost,
        swappiness: read_trim("/proc/sys/vm/swappiness")
            .and_then(|s| s.parse().ok())
            .unwrap_or(60),
        has_ppd,
    }
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn sysopt_set_profile(profile: String) -> Result<(), String> {
    let allowed = ["performance", "balanced", "power-saver"];
    if !allowed.contains(&profile.as_str()) {
        return Err("invalid profile".into());
    }
    let out = tauri::async_runtime::spawn_blocking(move || {
        Command::new("powerprofilesctl").args(["set", &profile]).output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

#[cfg(target_os = "linux")]
fn pkexec_write(pairs: Vec<(String, String)>) -> Result<(), String> {
    // one pkexec invocation for all writes → single auth dialog
    let script = pairs
        .iter()
        .map(|(v, p)| format!("echo {} > {}", v, p))
        .collect::<Vec<_>>()
        .join(" && ");
    let out = Command::new("pkexec")
        .args(["sh", "-c", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "authorization failed: {}",
            String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()
        ))
    }
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn sysopt_set_boost(on: bool) -> Result<(), String> {
    let paths = boost_paths();
    if paths.is_empty() {
        return Err("CPU boost control not exposed by this kernel".into());
    }
    let v = if on { "1" } else { "0" };
    let pairs: Vec<(String, String)> = paths.into_iter().map(|p| (v.to_string(), p)).collect();
    tauri::async_runtime::spawn_blocking(move || pkexec_write(pairs))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn sysopt_set_swappiness(value: u32) -> Result<(), String> {
    if value > 200 {
        return Err("swappiness must be 0..=200".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        pkexec_write(vec![(value.to_string(), "/proc/sys/vm/swappiness".into())])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Balance load across all CPU cores: online any offlined cores, apply one
/// governor uniformly, enable scheduler autogroup, spread IRQs when
/// irqbalance is available. One pkexec script → single auth dialog.
#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn sysopt_balance_cores() -> Result<String, String> {
    const SCRIPT: &str = r#"
onlined=0
for f in /sys/devices/system/cpu/cpu*/online; do
  [ -f "$f" ] || continue
  if [ "$(cat "$f")" = "0" ]; then echo 1 > "$f" && onlined=$((onlined+1)); fi
done
gov=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo none)
if [ "$gov" != "none" ]; then
  for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo "$gov" > "$g" 2>/dev/null || true; done
fi
autogroup=off
if [ -f /proc/sys/kernel/sched_autogroup_enabled ]; then
  echo 1 > /proc/sys/kernel/sched_autogroup_enabled && autogroup=on
fi
irq=absent
if command -v irqbalance >/dev/null 2>&1; then
  irqbalance --oneshot >/dev/null 2>&1 && irq=rebalanced || irq=failed
fi
echo "cores onlined: $onlined | governor: $gov (all cores) | autogroup: $autogroup | IRQ spread: $irq"
"#;
    tauri::async_runtime::spawn_blocking(|| {
        let out = Command::new("pkexec")
            .args(["sh", "-c", SCRIPT])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(format!(
                "authorization failed: {}",
                String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()
            ))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn sysopt_drop_caches() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let out = Command::new("pkexec")
            .args(["sh", "-c", "sync && echo 3 > /proc/sys/vm/drop_caches"])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err("authorization failed".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- macOS stubs (not applicable) ----------

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn sysopt_get() -> SysoptInfo {
    SysoptInfo {
        profile: String::new(),
        profiles: Vec::new(),
        governor: String::new(),
        epp: None,
        boost: None,
        swappiness: 0,
        has_ppd: false,
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn sysopt_set_profile(_profile: String) -> Result<(), String> {
    Err("power profiles not available on macOS — use System Settings → Battery".into())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn sysopt_set_boost(_on: bool) -> Result<(), String> {
    Err("CPU boost control not available on macOS".into())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn sysopt_set_swappiness(_value: u32) -> Result<(), String> {
    Err("swappiness control not available on macOS".into())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn sysopt_balance_cores() -> Result<String, String> {
    Err("core balancing not available on macOS — the kernel handles this automatically".into())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn sysopt_drop_caches() -> Result<(), String> {
    Err("cache dropping not available on macOS".into())
}
