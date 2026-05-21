use crate::{
    error::{AppError, Result},
    executor::{build_start_execution_plan, ExecutionAuth},
    models::RootJson,
    routes::{execute_query_payload, execute_steps, QueryRequest, QueryResponse},
    state::{model_asset_paths, set_model_asset_path, AppState},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::{path::Path, sync::Arc};
use tauri::State;
use tokio::sync::RwLock;
use tracing::info;

pub struct DesktopState {
    app: RwLock<Option<Arc<AppState>>>,
    last_error: RwLock<Option<String>>,
}

impl DesktopState {
    pub fn empty() -> Self {
        Self {
            app: RwLock::new(None),
            last_error: RwLock::new(None),
        }
    }
}

#[derive(Serialize)]
pub struct InitStatus {
    initialized: bool,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct UiLogResponse {
    next_cursor: u64,
    entries: Vec<UiLogLine>,
}

#[derive(Serialize)]
pub struct UiLogLine {
    id: u64,
    line: String,
}

#[derive(Serialize)]
pub struct ModelUploadResponse {
    target: String,
    path: String,
    filename: String,
}

#[derive(Serialize)]
pub struct ModelPathResponse {
    target: String,
    path: String,
}

#[derive(Serialize)]
pub struct SimpleStatusResponse {
    status: &'static str,
    message: String,
}

async fn set_last_error(state: &DesktopState, error: Option<String>) {
    let mut last_error = state.last_error.write().await;
    *last_error = error;
}

async fn app_state(state: &DesktopState) -> std::result::Result<Arc<AppState>, String> {
    if let Some(app) = state.app.read().await.clone() {
        return Ok(app);
    }

    Err("Load an agent JSON before running queries".to_string())
}

#[tauri::command]
pub async fn init_status(
    state: State<'_, DesktopState>,
) -> std::result::Result<InitStatus, String> {
    let initialized = state.app.read().await.is_some();
    let error = state.last_error.read().await.clone();
    Ok(InitStatus { initialized, error })
}

#[tauri::command]
pub async fn query(
    state: State<'_, DesktopState>,
    req: QueryRequest,
) -> std::result::Result<QueryResponse, String> {
    let app = app_state(state.inner()).await?;
    execute_query_payload(app, req)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn load_agent(
    state: State<'_, DesktopState>,
    config: RootJson,
    elevated_password: Option<String>,
) -> std::result::Result<SimpleStatusResponse, String> {
    let start = config.start.clone();
    let mut app_lock = state.app.write().await;

    if let Some(app) = app_lock.as_ref() {
        app.update_agent_config(config)
            .await
            .map_err(|err| err.to_string())?;
    } else {
        let app = AppState::from_config(config)
            .await
            .map_err(|err| err.to_string())?;
        *app_lock = Some(Arc::new(app));
    }

    let app = app_lock
        .as_ref()
        .cloned()
        .ok_or_else(|| "Agent failed to initialize".to_string())?;
    drop(app_lock);

    if let Some(start) = start.as_ref() {
        let steps = build_start_execution_plan(start).map_err(|err| err.to_string())?;
        if !steps.is_empty() {
            let auth = ExecutionAuth { elevated_password };
            execute_steps(app, steps, &auth)
                .await
                .map_err(|err| err.to_string())?;
        }
    }

    set_last_error(state.inner(), None).await;
    Ok(SimpleStatusResponse {
        status: "success",
        message: "Agent loaded successfully".to_string(),
    })
}

#[tauri::command]
pub async fn reload_models(
    state: State<'_, DesktopState>,
) -> std::result::Result<SimpleStatusResponse, String> {
    let app = app_state(state.inner()).await?;
    app.reload_models().await.map_err(|err| err.to_string())?;
    Ok(SimpleStatusResponse {
        status: "success",
        message: "Models reloaded successfully".to_string(),
    })
}

#[tauri::command]
pub async fn logs(since: Option<u64>) -> std::result::Result<UiLogResponse, String> {
    let (next_cursor, entries) = crate::ui_logs::read_logs(since);
    Ok(UiLogResponse {
        next_cursor,
        entries: entries
            .into_iter()
            .map(|entry| UiLogLine {
                id: entry.id,
                line: entry.line,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn upload_model_file(
    state: State<'_, DesktopState>,
    target: String,
    filename: Option<String>,
    data_base64: String,
) -> std::result::Result<ModelUploadResponse, String> {
    let paths = model_asset_paths();
    let path = match target.as_str() {
        "gte_model" => paths.gte_model,
        "gte_tokenizer" => paths.gte_tokenizer,
        "gliner_model" => paths.gliner_model,
        "gliner_tokenizer" => paths.gliner_tokenizer,
        other => return Err(format!("Unknown model upload target: {}", other)),
    };

    let filename = filename
        .as_deref()
        .map(sanitize_filename)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "uploaded-file".to_string());
    let bytes = decode_base64(&data_base64)?;

    write_bytes_to_path(bytes, Path::new(&path))
        .await
        .map_err(|err| err.to_string())?;
    info!(" Replaced {}", target);

    if let Some(app) = state.app.read().await.clone() {
        app.reload_models().await.map_err(|err| err.to_string())?;
    }

    Ok(ModelUploadResponse {
        target,
        path,
        filename,
    })
}

#[tauri::command]
pub async fn use_model_file_path(
    state: State<'_, DesktopState>,
    target: String,
    path: String,
) -> std::result::Result<ModelPathResponse, String> {
    let path = set_model_asset_path(&target, Path::new(&path).to_path_buf())
        .map_err(|err| err.to_string())?;
    info!(" Using {} from {}", target, path);

    if let Some(app) = state.app.read().await.clone() {
        app.reload_models().await.map_err(|err| err.to_string())?;
    }

    Ok(ModelPathResponse { target, path })
}

async fn write_bytes_to_path(bytes: Vec<u8>, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create model directory: {}", e))
        })?;
    }

    let temp_path = path.with_extension("uploading");
    tokio::fs::write(&temp_path, bytes).await.map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to write uploaded file '{}': {}",
            temp_path.display(),
            e
        ))
    })?;
    tokio::fs::rename(&temp_path, path).await.map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to replace model file '{}': {}",
            path.display(),
            e
        ))
    })?;
    Ok(())
}

fn decode_base64(value: &str) -> std::result::Result<Vec<u8>, String> {
    STANDARD
        .decode(value)
        .map_err(|e| format!("Invalid base64 payload: {}", e))
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}
