use dotenvy::dotenv;
use quicktasks::state::configure_runtime_paths;
use quicktasks::tauri_commands::{self, DesktopState};
use tauri::Manager;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    dotenv().ok();

    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "quicktasks=debug".into()),
        )
        .with(quicktasks::ui_logs::layer())
        .with(tracing_subscriber::fmt::layer())
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_local_data_dir()?;
            let resource_dir = app.path().resource_dir().ok();

            configure_runtime_paths(app_data_dir, resource_dir)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

            Ok(())
        })
        .manage(DesktopState::empty())
        .invoke_handler(tauri::generate_handler![
            tauri_commands::init_status,
            tauri_commands::query,
            tauri_commands::load_agent,
            tauri_commands::reload_models,
            tauri_commands::logs,
            tauri_commands::upload_model_file,
            tauri_commands::use_model_file_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run QuickTasks desktop app");
}
