#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod bench;
mod sysopt;
mod telemetry;
mod vault;

use std::sync::Mutex;

/// Lets the frontend adapt to the platform (e.g. hide Linux-only tabs).
#[tauri::command]
fn app_os() -> &'static str {
    std::env::consts::OS
}

fn main() {
    tauri::Builder::default()
        .manage(telemetry::TelemetryState(Mutex::new(Default::default())))
        .manage(vault::VaultState(Mutex::new(vault::open_db())))
        .manage(ai::AiState {
            config: Mutex::new(ai::load_config()),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .expect("build reqwest client"),
        })
        .setup(|app| {
            telemetry::spawn(app.handle().clone());
            vault::spawn_watcher(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_os,
            telemetry::telemetry_snapshot,
            vault::vault_list,
            vault::vault_get,
            vault::vault_delete,
            vault::vault_pin,
            vault::vault_wipe,
            vault::vault_copy,
            vault::vault_add_text,
            vault::vault_add_image,
            vault::vault_add_path,
            vault::vault_add_audio,
            vault::vault_save_as,
            vault::vault_stats,
            ai::ai_get_config,
            ai::ai_set_config,
            ai::ai_test,
            ai::ai_run,
            ai::ai_chat,
            ai::ai_transcribe,
            ai::ai_optimize_generate,
            ai::ai_optimize_apply,
            bench::bench_run,
            sysopt::sysopt_get,
            sysopt::sysopt_set_profile,
            sysopt::sysopt_set_boost,
            sysopt::sysopt_set_swappiness,
            sysopt::sysopt_balance_cores,
            sysopt::sysopt_drop_caches,
        ])
        .run(tauri::generate_context!())
        .expect("error while running aura pulse");
}
