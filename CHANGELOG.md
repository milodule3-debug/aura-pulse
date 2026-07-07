# Changelog

## 0.4.1 — macOS support

- Native macOS build (Apple Silicon, `.dmg`) alongside Linux and Windows.
- CI: dedicated `build-macos` job with `aarch64-apple-darwin` target; Linux, macOS
  and Windows now build in parallel and a single `release` job attaches all
  installers to one GitHub Release.
- Fixes required to get there: cross-platform `sysopt` stubs (previously
  macOS-only), removal of the `rfd` file-dialog crate (native save-as now
  writes to the Downloads directory), and a process-spawn path fix
  (`macos_imp` → `imp`) so the packaged binary can find its helper on macOS.

## 0.3.0 — Windows support, optimization engine, audio AI

- Windows installer (NSIS + MSI) via CI.
- File ingestion and audio AI enrichment in the Vault.
- System optimization engine (power profiles, CPU boost, cache dropping).

## 0.1.0 — Initial release

- First public release: system telemetry dashboard, AI clipboard vault,
  hardware benchmarks, Tron-style UI. Linux only, MIT licensed, npm
  distribution.

---

See [docs/ROADMAP.md](docs/ROADMAP.md) for the original historical feature
brief this project was built from.
