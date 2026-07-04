# Building Aura Pulse for Windows 11

The codebase is cross-platform as of 0.3.0: the Rust backend compiles for
Windows with a `sysinfo`-based telemetry sampler, and the frontend hides the
Linux-only Optimization tab at runtime (`app_os` command).

## What works on Windows

| Feature | Status |
|---|---|
| The Vault (clipboard history, drag-drop, click-to-browse) | ✅ full — arboard supports Windows |
| AI Core hub (all providers) | ✅ full |
| AI enrichment (summarize / OCR / describe / markdown / design) | ✅ full |
| Audio transcription | ✅ Gemini path; whisper.cpp is found in `%USERPROFILE%\whisper.cpp` if built |
| Benchmarks (CPU / memory / disk / LLM estimate + Ollama & LM Studio probes) | ✅ full |
| Diagnostics | ⚠️ CPU %, per-core, memory, swap, disks, network, top processes, uptime work. CPU watts (RAPL), GPU sysfs telemetry and battery watts are Linux-only and read as 0 |
| Optimization tab (power profiles, boost, swappiness, AI modules) | ❌ hidden on Windows — pkexec / sysfs / power-profiles-daemon have no Windows counterpart yet |

## Option A — build on a Windows machine (recommended)

Prereqs (one-time):
1. Install [Node.js ≥ 18](https://nodejs.org), [Rust](https://rustup.rs) (stable-msvc, the default), and
   [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   with the "Desktop development with C++" workload.
2. WebView2 runtime ships with Windows 11 — nothing to install.

Build:

```powershell
git clone <repo> aura-pulse && cd aura-pulse
npm ci
npm run tauri build
```

Installers land in:
- `src-tauri\target\release\bundle\nsis\Aura Pulse_<ver>_x64-setup.exe`  ← the one to distribute
- `src-tauri\target\release\bundle\msi\Aura Pulse_<ver>_x64_en-US.msi`

Both register Start-menu entries and an uninstaller.

## Option B — GitHub Actions (no Windows machine needed)

`.github/workflows/windows-build.yml` builds the NSIS `.exe` and `.msi` on a
real Windows runner:

- push a tag like `v0.3.0` → installers attach to the GitHub release, or
- run the workflow manually (workflow_dispatch) → download from the run's artifacts.

Setup: push this repo to GitHub, then `git tag v0.3.0 && git push --tags`.

## Option C — cross-compile from Linux (experimental)

Only the NSIS bundler works cross-platform, via the MSVC target and
`cargo-xwin` (downloads the Windows SDK headers):

```bash
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
sudo apt install nsis lld
npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
```

Known caveats: rusqlite's bundled SQLite and the `ring` crypto crate usually
build under xwin, but toolchain drift breaks this path regularly, and the
result can't be smoke-tested here. Prefer A or B for anything you distribute.

## Code signing (later)

The installers are unsigned; Windows SmartScreen will warn on first run
("More info → Run anyway"). For public distribution get a code-signing
certificate (EV for instant reputation) and configure
`tauri.conf.json > bundle > windows > certificateThumbprint`.
