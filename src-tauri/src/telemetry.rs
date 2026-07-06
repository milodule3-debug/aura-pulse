//! High-frequency system telemetry: 10 Hz sampling thread.
//! Linux reads /proc and /sys directly (no shelling out in the hot path);
//! Windows samples through the `sysinfo` crate (less depth: no RAPL watts,
//! no GPU sysfs, no per-disk IO rates — those fields degrade to zero).
//! macOS samples through the `sysinfo` crate plus sysctl for freq/temp.

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

// ---------- snapshot model ----------

#[derive(Serialize, Clone, Default)]
pub struct Cpu {
    pub total: f32,
    pub cores: Vec<f32>,
    pub freq_mhz: f32,
    pub temp: f32,
}

#[derive(Serialize, Clone, Default)]
pub struct Mem {
    pub total: u64,
    pub used: u64,
    pub avail: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct Gpu {
    pub present: bool,
    pub busy: f32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub temp: f32,
    pub watts: f32,
}

#[derive(Serialize, Clone, Default)]
pub struct Power {
    pub cpu_watts: f32,
    pub batt_watts: f32,
    pub batt_pct: f32,
    pub on_ac: bool,
    pub profile: String,
}

#[derive(Serialize, Clone, Default)]
pub struct MountInfo {
    pub path: String,
    pub total: u64,
    pub used: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct DiskIo {
    pub mounts: Vec<MountInfo>,
    pub read_bps: f64,
    pub write_bps: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct Net {
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct SysMeta {
    pub load1: f32,
    pub load5: f32,
    pub uptime: u64,
    pub procs: u32,
}

#[derive(Serialize, Clone, Default)]
pub struct TopProc {
    pub pid: i32,
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct Snapshot {
    pub ts: u64,
    pub cpu: Cpu,
    pub mem: Mem,
    pub gpu: Gpu,
    pub power: Power,
    pub disk: DiskIo,
    pub net: Net,
    pub sys: SysMeta,
    pub top: Vec<TopProc>,
}

pub struct TelemetryState(pub Mutex<Snapshot>);

#[tauri::command]
pub fn telemetry_snapshot(state: tauri::State<'_, TelemetryState>) -> Snapshot {
    state.0.lock().unwrap().clone()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn publish(app: &AppHandle, snap: Snapshot) {
    if let Some(state) = app.try_state::<TelemetryState>() {
        *state.0.lock().unwrap() = snap.clone();
    }
    let _ = app.emit("telemetry", &snap);
}

pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("telemetry".into())
        .spawn(move || imp::run(app))
        .expect("spawn telemetry thread");
}

// ---------- Linux backend: raw /proc + /sys ----------

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn read_str(path: &str) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    fn read_f64(path: &str) -> Option<f64> {
        read_str(path)?.parse().ok()
    }

    fn find_hwmon(name: &str) -> Option<PathBuf> {
        let entries = fs::read_dir("/sys/class/hwmon").ok()?;
        for e in entries.flatten() {
            let p = e.path();
            if let Ok(n) = fs::read_to_string(p.join("name")) {
                if n.trim() == name {
                    return Some(p);
                }
            }
        }
        None
    }

    fn find_gpu_device() -> Option<PathBuf> {
        let entries = fs::read_dir("/sys/class/drm").ok()?;
        for e in entries.flatten() {
            let dev = e.path().join("device");
            if dev.join("gpu_busy_percent").exists() {
                return Some(dev);
            }
        }
        None
    }

    struct CpuTimes {
        busy: u64,
        total: u64,
    }

    fn read_cpu_times() -> (CpuTimes, Vec<CpuTimes>) {
        let mut agg = CpuTimes { busy: 0, total: 0 };
        let mut cores = Vec::new();
        if let Ok(s) = fs::read_to_string("/proc/stat") {
            for line in s.lines() {
                if !line.starts_with("cpu") {
                    break;
                }
                let f: Vec<u64> = line
                    .split_whitespace()
                    .skip(1)
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if f.len() < 5 {
                    continue;
                }
                let total: u64 = f.iter().sum();
                let idle = f[3] + f.get(4).copied().unwrap_or(0);
                let t = CpuTimes { busy: total - idle, total };
                if line.starts_with("cpu ") {
                    agg = t;
                } else {
                    cores.push(t);
                }
            }
        }
        (agg, cores)
    }

    fn cpu_pct(prev: &CpuTimes, cur: &CpuTimes) -> f32 {
        let dt = cur.total.saturating_sub(prev.total);
        if dt == 0 {
            return 0.0;
        }
        (cur.busy.saturating_sub(prev.busy) as f32 / dt as f32 * 100.0).clamp(0.0, 100.0)
    }

    fn read_meminfo() -> Mem {
        let mut m = HashMap::new();
        if let Ok(s) = fs::read_to_string("/proc/meminfo") {
            for line in s.lines() {
                let mut it = line.split_whitespace();
                if let (Some(k), Some(v)) = (it.next(), it.next()) {
                    if let Ok(kb) = v.parse::<u64>() {
                        m.insert(k.trim_end_matches(':').to_string(), kb * 1024);
                    }
                }
            }
        }
        let total = *m.get("MemTotal").unwrap_or(&0);
        let avail = *m.get("MemAvailable").unwrap_or(&0);
        let swap_total = *m.get("SwapTotal").unwrap_or(&0);
        let swap_free = *m.get("SwapFree").unwrap_or(&0);
        Mem {
            total,
            used: total.saturating_sub(avail),
            avail,
            swap_total,
            swap_used: swap_total.saturating_sub(swap_free),
        }
    }

    fn avg_freq_mhz(ncores: usize) -> f32 {
        let mut sum = 0.0;
        let mut n = 0;
        for i in 0..ncores {
            if let Some(khz) = read_f64(&format!(
                "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq",
                i
            )) {
                sum += khz / 1000.0;
                n += 1;
            }
        }
        if n > 0 {
            (sum / n as f64) as f32
        } else {
            0.0
        }
    }

    fn is_whole_disk(name: &str) -> bool {
        if let Some(rest) = name.strip_prefix("nvme") {
            return rest.contains('n') && !rest.contains('p');
        }
        if name.starts_with("sd") || name.starts_with("vd") {
            return name.chars().last().map(|c| c.is_alphabetic()).unwrap_or(false);
        }
        if let Some(rest) = name.strip_prefix("mmcblk") {
            return !rest.contains('p');
        }
        false
    }

    fn read_disk_sectors() -> (u64, u64) {
        let (mut rd, mut wr) = (0u64, 0u64);
        if let Ok(s) = fs::read_to_string("/proc/diskstats") {
            for line in s.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 10 || !is_whole_disk(f[2]) {
                    continue;
                }
                rd += f[5].parse::<u64>().unwrap_or(0);
                wr += f[9].parse::<u64>().unwrap_or(0);
            }
        }
        (rd * 512, wr * 512)
    }

    fn read_net_bytes() -> (u64, u64) {
        let (mut rx, mut tx) = (0u64, 0u64);
        if let Ok(s) = fs::read_to_string("/proc/net/dev") {
            for line in s.lines().skip(2) {
                let mut parts = line.split(':');
                let iface = parts.next().unwrap_or("").trim();
                if iface == "lo" || iface.is_empty() {
                    continue;
                }
                let f: Vec<u64> = parts
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if f.len() >= 9 {
                    rx += f[0];
                    tx += f[8];
                }
            }
        }
        (rx, tx)
    }

    fn statvfs(path: &str) -> Option<(u64, u64)> {
        use std::ffi::CString;
        let c = CString::new(path).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0 {
            let frsize = s.f_frsize as u64;
            let total = s.f_blocks as u64 * frsize;
            let used = total.saturating_sub(s.f_bfree as u64 * frsize);
            Some((total, used))
        } else {
            None
        }
    }

    fn discover_mounts() -> Vec<String> {
        let ok_fs = ["ext4", "ext3", "ext2", "btrfs", "xfs", "f2fs", "vfat", "ntfs", "exfat"];
        let mut seen_dev = Vec::new();
        let mut out = Vec::new();
        if let Ok(s) = fs::read_to_string("/proc/mounts") {
            for line in s.lines() {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 3 || !f[0].starts_with("/dev/") || !ok_fs.contains(&f[2]) {
                    continue;
                }
                if seen_dev.contains(&f[0].to_string()) {
                    continue;
                }
                seen_dev.push(f[0].to_string());
                out.push(f[1].replace("\\040", " "));
                if out.len() >= 4 {
                    break;
                }
            }
        }
        if out.is_empty() {
            out.push("/".into());
        }
        out
    }

    fn read_battery() -> (f32, f32, bool) {
        let base = "/sys/class/power_supply/BAT0";
        let pct = read_f64(&format!("{}/capacity", base)).unwrap_or(0.0) as f32;
        let watts = read_f64(&format!("{}/power_now", base))
            .map(|uw| uw / 1e6)
            .or_else(|| {
                let v = read_f64(&format!("{}/voltage_now", base))?;
                let c = read_f64(&format!("{}/current_now", base))?;
                Some(v * c / 1e12)
            })
            .unwrap_or(0.0) as f32;
        let on_ac = read_str("/sys/class/power_supply/ADP1/online")
            .or_else(|| read_str("/sys/class/power_supply/AC/online"))
            .map(|s| s == "1")
            .unwrap_or(true);
        (pct, watts, on_ac)
    }

    struct ProcTracker {
        prev: HashMap<i32, u64>,
        prev_t: Instant,
        clk_tck: f64,
        page_size: u64,
    }

    impl ProcTracker {
        fn new() -> Self {
            Self {
                prev: HashMap::new(),
                prev_t: Instant::now(),
                clk_tck: unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64,
                page_size: unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64,
            }
        }

        fn sample(&mut self) -> Vec<TopProc> {
            let now = Instant::now();
            let dt = now.duration_since(self.prev_t).as_secs_f64().max(0.05);
            let mut cur: HashMap<i32, u64> = HashMap::new();
            let mut procs: Vec<(i32, String, u64)> = Vec::new();

            if let Ok(rd) = fs::read_dir("/proc") {
                for e in rd.flatten() {
                    let name = e.file_name();
                    let pid: i32 = match name.to_string_lossy().parse() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let stat = match fs::read_to_string(format!("/proc/{}/stat", pid)) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    // comm is inside parens and may contain spaces
                    let close = match stat.rfind(')') {
                        Some(i) => i,
                        None => continue,
                    };
                    let comm = stat[stat.find('(').map(|i| i + 1).unwrap_or(0)..close].to_string();
                    let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
                    if rest.len() < 13 {
                        continue;
                    }
                    let utime: u64 = rest[11].parse().unwrap_or(0);
                    let stime: u64 = rest[12].parse().unwrap_or(0);
                    cur.insert(pid, utime + stime);
                    procs.push((pid, comm, utime + stime));
                }
            }

            let mut top: Vec<TopProc> = procs
                .into_iter()
                .filter_map(|(pid, name, ticks)| {
                    let prev = *self.prev.get(&pid)?;
                    let d = ticks.saturating_sub(prev) as f64;
                    let cpu = (d / self.clk_tck / dt * 100.0) as f32;
                    if cpu < 0.1 {
                        return None;
                    }
                    let mem = fs::read_to_string(format!("/proc/{}/statm", pid))
                        .ok()
                        .and_then(|s| {
                            s.split_whitespace()
                                .nth(1)
                                .and_then(|v| v.parse::<u64>().ok())
                        })
                        .map(|pages| pages * self.page_size)
                        .unwrap_or(0);
                    Some(TopProc { pid, name, cpu, mem })
                })
                .collect();
            top.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
            top.truncate(8);

            self.prev = cur;
            self.prev_t = now;
            top
        }
    }

    pub fn run(app: AppHandle) {
        let cpu_temp_hwmon = find_hwmon("k10temp").or_else(|| find_hwmon("coretemp"));
        let gpu_hwmon = find_hwmon("amdgpu");
        let gpu_dev = find_gpu_device();
        let rapl_path = "/sys/class/powercap/intel-rapl:0/energy_uj";
        let rapl_max = read_f64("/sys/class/powercap/intel-rapl:0/max_energy_range_uj");
        let mounts = discover_mounts();

        // resolve which amdgpu power file exists once
        let gpu_power_file = gpu_hwmon.as_ref().and_then(|h| {
            for f in ["power1_average", "power1_input"] {
                let p = h.join(f);
                if p.exists() {
                    return Some(p);
                }
            }
            None
        });

        let (mut prev_agg, mut prev_cores) = read_cpu_times();
        let mut prev_rapl = read_f64(rapl_path);
        let (mut prev_dr, mut prev_dw) = read_disk_sectors();
        let (mut prev_rx, mut prev_tx) = read_net_bytes();
        let mut prev_io_t = Instant::now();

        let mut tracker = ProcTracker::new();
        let mut cached_top: Vec<TopProc> = Vec::new();
        let mut cached_profile = String::new();
        let mut tick: u64 = 0;

        loop {
            let loop_start = Instant::now();

            // CPU
            let (agg, cores) = read_cpu_times();
            let total = cpu_pct(&prev_agg, &agg);
            let core_pcts: Vec<f32> = cores
                .iter()
                .zip(prev_cores.iter())
                .map(|(c, p)| cpu_pct(p, c))
                .collect();
            let ncores = cores.len().max(1);
            prev_agg = agg;
            prev_cores = cores;

            let cpu_temp = cpu_temp_hwmon
                .as_ref()
                .and_then(|h| read_f64(h.join("temp1_input").to_str()?))
                .map(|v| (v / 1000.0) as f32)
                .unwrap_or(0.0);

            // GPU
            let gpu = if let Some(dev) = &gpu_dev {
                let busy = read_f64(dev.join("gpu_busy_percent").to_str().unwrap_or("")).unwrap_or(0.0);
                let vu = read_f64(dev.join("mem_info_vram_used").to_str().unwrap_or("")).unwrap_or(0.0);
                let vt = read_f64(dev.join("mem_info_vram_total").to_str().unwrap_or("")).unwrap_or(0.0);
                let temp = gpu_hwmon
                    .as_ref()
                    .and_then(|h| read_f64(h.join("temp1_input").to_str()?))
                    .map(|v| v / 1000.0)
                    .unwrap_or(0.0);
                let watts = gpu_power_file
                    .as_ref()
                    .and_then(|p| read_f64(p.to_str()?))
                    .map(|uw| uw / 1e6)
                    .unwrap_or(0.0);
                Gpu {
                    present: true,
                    busy: busy as f32,
                    vram_used: vu as u64,
                    vram_total: vt as u64,
                    temp: temp as f32,
                    watts: watts as f32,
                }
            } else {
                Gpu::default()
            };

            // Power: RAPL package watts
            let dt_io = prev_io_t.elapsed().as_secs_f64().max(0.01);
            let cpu_watts = if let (Some(prev), Some(cur)) = (prev_rapl, read_f64(rapl_path)) {
                let mut d = cur - prev;
                if d < 0.0 {
                    d += rapl_max.unwrap_or(0.0);
                }
                prev_rapl = Some(cur);
                (d / 1e6 / dt_io) as f32
            } else {
                0.0
            };

            // Disk + Net rates (measured against real elapsed time)
            let (dr, dw) = read_disk_sectors();
            let (rx, tx) = read_net_bytes();
            let read_bps = (dr.saturating_sub(prev_dr)) as f64 / dt_io;
            let write_bps = (dw.saturating_sub(prev_dw)) as f64 / dt_io;
            let rx_bps = (rx.saturating_sub(prev_rx)) as f64 / dt_io;
            let tx_bps = (tx.saturating_sub(prev_tx)) as f64 / dt_io;
            prev_dr = dr;
            prev_dw = dw;
            prev_rx = rx;
            prev_tx = tx;
            prev_io_t = Instant::now();

            // Meta
            let (load1, load5, procs) = read_str("/proc/loadavg")
                .map(|s| {
                    let f: Vec<&str> = s.split_whitespace().collect();
                    let l1 = f.first().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let l5 = f.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let pr = f
                        .get(3)
                        .and_then(|v| v.split('/').nth(1))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    (l1, l5, pr)
                })
                .unwrap_or((0.0, 0.0, 0));
            let uptime = read_str("/proc/uptime")
                .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
                .unwrap_or(0.0) as u64;
            let (batt_pct, batt_watts, on_ac) = read_battery();

            // Slow lane: every 2 s refresh top procs + power profile
            if tick.is_multiple_of(20) {
                cached_top = tracker.sample();
                cached_profile = Command::new("powerprofilesctl")
                    .arg("get")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
            }

            let snap = Snapshot {
                ts: now_ms(),
                cpu: Cpu {
                    total,
                    cores: core_pcts,
                    freq_mhz: avg_freq_mhz(ncores),
                    temp: cpu_temp,
                },
                mem: read_meminfo(),
                gpu,
                power: Power {
                    cpu_watts,
                    batt_watts,
                    batt_pct,
                    on_ac,
                    profile: cached_profile.clone(),
                },
                disk: DiskIo {
                    mounts: mounts
                        .iter()
                        .filter_map(|m| {
                            let (total, used) = statvfs(m)?;
                            Some(MountInfo { path: m.clone(), total, used })
                        })
                        .collect(),
                    read_bps,
                    write_bps,
                },
                net: Net { rx_bps, tx_bps },
                sys: SysMeta { load1, load5, uptime, procs },
                top: cached_top.clone(),
            };

            publish(&app, snap);

            tick += 1;
            let elapsed = loop_start.elapsed();
            if elapsed < Duration::from_millis(100) {
                std::thread::sleep(Duration::from_millis(100) - elapsed);
            }
        }
    }
}

// ---------- macOS backend: sysinfo crate + sysctl ----------

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use sysinfo::{Components, Disks, Networks, ProcessesToUpdate, System};

    fn sysctl_f64(name: &str) -> Option<f64> {
        let out = std::process::Command::new("sysctl").arg("-n").arg(name).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }

    pub fn run(app: AppHandle) {
        let mut sys = System::new_all();
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let mut components = Components::new_with_refreshed_list();

        let mut cached_top: Vec<TopProc> = Vec::new();
        let mut cached_mounts: Vec<MountInfo> = Vec::new();
        let mut cached_procs: u32 = 0;
        let mut cpu_temp: f32 = 0.0;
        let mut prev_net_t = Instant::now();
        let mut tick: u64 = 0;

        loop {
            let loop_start = Instant::now();

            sys.refresh_cpu_usage();
            sys.refresh_memory();

            let dt = prev_net_t.elapsed().as_secs_f64().max(0.01);
            networks.refresh(true);
            prev_net_t = Instant::now();
            let (mut rx, mut tx) = (0u64, 0u64);
            for (_name, data) in networks.iter() {
                rx += data.received();
                tx += data.transmitted();
            }

            // Slow lane: every 2 s refresh processes, disks, temps, battery
            if tick.is_multiple_of(20) {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                cached_procs = sys.processes().len() as u32;
                let mut top: Vec<TopProc> = sys
                    .processes()
                    .values()
                    .filter(|p| p.cpu_usage() >= 0.1)
                    .map(|p| TopProc {
                        pid: p.pid().as_u32() as i32,
                        name: p.name().to_string_lossy().into_owned(),
                        cpu: p.cpu_usage(),
                        mem: p.memory(),
                    })
                    .collect();
                top.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
                top.truncate(8);
                cached_top = top;

                disks.refresh(true);
                cached_mounts = disks
                    .iter()
                    .take(4)
                    .map(|d| {
                        let total = d.total_space();
                        MountInfo {
                            path: d.mount_point().to_string_lossy().into_owned(),
                            total,
                            used: total.saturating_sub(d.available_space()),
                        }
                    })
                    .collect();

                components.refresh(true);
                cpu_temp = components
                    .iter()
                    .find(|c| {
                        let l = c.label().to_lowercase();
                        l.contains("cpu") || l.contains("package") || l.contains("die")
                    })
                    .and_then(|c| c.temperature())
                    .unwrap_or(0.0);
            }

            // CPU frequency via sysctl (macOS hw.cpufrequency is in Hz)
            let freq_mhz = sysctl_f64("hw.cpufrequency")
                .map(|hz| hz / 1e6)
                .unwrap_or_else(|| {
                    let cpus = sys.cpus();
                    if cpus.is_empty() { 0.0 }
                    else { cpus.iter().map(|c| c.frequency() as f64).sum::<f64>() / cpus.len() as f64 }
                }) as f32;

            let load = System::load_average();

            // Battery via pmset
            let (batt_pct, batt_watts, on_ac) = read_macos_battery();

            let snap = Snapshot {
                ts: now_ms(),
                cpu: Cpu {
                    total: sys.global_cpu_usage(),
                    cores: sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
                    freq_mhz,
                    temp: cpu_temp,
                },
                mem: Mem {
                    total: sys.total_memory(),
                    used: sys.total_memory().saturating_sub(sys.available_memory()),
                    avail: sys.available_memory(),
                    swap_total: sys.total_swap(),
                    swap_used: sys.used_swap(),
                },
                gpu: Gpu::default(),
                power: Power {
                    cpu_watts: 0.0,
                    batt_watts,
                    batt_pct,
                    on_ac,
                    profile: String::new(),
                },
                disk: DiskIo {
                    mounts: cached_mounts.clone(),
                    read_bps: 0.0,
                    write_bps: 0.0,
                },
                net: Net {
                    rx_bps: rx as f64 / dt,
                    tx_bps: tx as f64 / dt,
                },
                sys: SysMeta {
                    load1: load.one as f32,
                    load5: load.five as f32,
                    uptime: System::uptime(),
                    procs: cached_procs,
                },
                top: cached_top.clone(),
            };

            publish(&app, snap);

            tick += 1;
            let elapsed = loop_start.elapsed();
            if elapsed < Duration::from_millis(100) {
                std::thread::sleep(Duration::from_millis(100) - elapsed);
            }
        }
    }

    fn read_macos_battery() -> (f32, f32, bool) {
        let out = std::process::Command::new("pmset").arg("-g").arg("batt").output();
        let text = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return (100.0, 0.0, true),
        };
        let on_ac = text.contains("AC attached") || text.contains("charging");
        let pct = text
            .find('%')
            .and_then(|i| text[..i].rfind(|c: char| !c.is_ascii_digit()).map(|j| &text[j + 1..i + 1]))
            .and_then(|s| s.trim_end_matches('%').parse::<f32>().ok())
            .unwrap_or(100.0);
        (pct, 0.0, on_ac)
    }
}

// ---------- Windows backend: sysinfo crate ----------

#[cfg(windows)]
mod imp {
    use super::*;
    use sysinfo::{Components, Disks, Networks, ProcessesToUpdate, System};

    pub fn run(app: AppHandle) {
        let mut sys = System::new_all();
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let mut components = Components::new_with_refreshed_list();

        let mut cached_top: Vec<TopProc> = Vec::new();
        let mut cached_mounts: Vec<MountInfo> = Vec::new();
        let mut cached_procs: u32 = 0;
        let mut cpu_temp: f32 = 0.0;
        let mut prev_net_t = Instant::now();
        let mut tick: u64 = 0;

        loop {
            let loop_start = Instant::now();

            sys.refresh_cpu_usage();
            sys.refresh_memory();

            let dt = prev_net_t.elapsed().as_secs_f64().max(0.01);
            networks.refresh(true);
            prev_net_t = Instant::now();
            let (mut rx, mut tx) = (0u64, 0u64);
            for (_name, data) in networks.iter() {
                rx += data.received();
                tx += data.transmitted();
            }

            // Slow lane: every 2 s refresh processes, disks and temps
    if tick.is_multiple_of(20) {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                cached_procs = sys.processes().len() as u32;
                let mut top: Vec<TopProc> = sys
                    .processes()
                    .values()
                    .filter(|p| p.cpu_usage() >= 0.1)
                    .map(|p| TopProc {
                        pid: p.pid().as_u32() as i32,
                        name: p.name().to_string_lossy().into_owned(),
                        cpu: p.cpu_usage(),
                        mem: p.memory(),
                    })
                    .collect();
                top.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
                top.truncate(8);
                cached_top = top;

                disks.refresh(true);
                cached_mounts = disks
                    .iter()
                    .take(4)
                    .map(|d| {
                        let total = d.total_space();
                        MountInfo {
                            path: d.mount_point().to_string_lossy().into_owned(),
                            total,
                            used: total.saturating_sub(d.available_space()),
                        }
                    })
                    .collect();

                components.refresh(true);
                cpu_temp = components
                    .iter()
                    .find(|c| {
                        let l = c.label().to_lowercase();
                        l.contains("cpu") || l.contains("package")
                    })
                    .and_then(|c| c.temperature())
                    .unwrap_or(0.0);
            }

            let cpus = sys.cpus();
            let freq_mhz = if cpus.is_empty() {
                0.0
            } else {
                cpus.iter().map(|c| c.frequency() as f32).sum::<f32>() / cpus.len() as f32
            };
            let load = System::load_average(); // zeros on Windows — shown as such

            let snap = Snapshot {
                ts: now_ms(),
                cpu: Cpu {
                    total: sys.global_cpu_usage(),
                    cores: cpus.iter().map(|c| c.cpu_usage()).collect(),
                    freq_mhz,
                    temp: cpu_temp,
                },
                mem: Mem {
                    total: sys.total_memory(),
                    used: sys.total_memory().saturating_sub(sys.available_memory()),
                    avail: sys.available_memory(),
                    swap_total: sys.total_swap(),
                    swap_used: sys.used_swap(),
                },
                gpu: Gpu::default(), // no portable GPU telemetry yet
                power: Power {
                    cpu_watts: 0.0,
                    batt_watts: 0.0,
                    batt_pct: 100.0,
                    on_ac: true,
                    profile: String::new(),
                },
                disk: DiskIo {
                    mounts: cached_mounts.clone(),
                    read_bps: 0.0, // sysinfo has no global disk IO counters
                    write_bps: 0.0,
                },
                net: Net {
                    rx_bps: rx as f64 / dt,
                    tx_bps: tx as f64 / dt,
                },
                sys: SysMeta {
                    load1: load.one as f32,
                    load5: load.five as f32,
                    uptime: System::uptime(),
                    procs: cached_procs,
                },
                top: cached_top.clone(),
            };

            publish(&app, snap);

            tick += 1;
            let elapsed = loop_start.elapsed();
            if elapsed < Duration::from_millis(100) {
                std::thread::sleep(Duration::from_millis(100) - elapsed);
            }
        }
    }
}
