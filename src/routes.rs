#[cfg(feature = "ui")]
use crate::state::model_asset_paths;
use crate::{
    error::{AppError, Result},
    executor::{
        build_execution_plan, execute_command, execute_http, ExecutionAuth, ExecutionPlan,
        ExecutionStep,
    },
    models::{
        FileInput, OutputPayload, Param, ParamKind, ParamValue, ParamValues, RequestParamValue,
    },
    params::extract_params,
    routing::route,
    state::AppState,
};
#[cfg(feature = "ui")]
use axum::{
    body::Bytes,
    extract::{Query, Request},
};
use axum::{extract::State, Json};
use base64::{engine::general_purpose::STANDARD, Engine as _};
#[cfg(feature = "ui")]
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
#[cfg(feature = "ui")]
use std::path::Path;
use std::time::Instant;
use std::{collections::HashMap, sync::Arc};
#[cfg(feature = "ui")]
use tokio::io::AsyncWriteExt;
use tracing::{debug, info};

#[derive(Deserialize)]
pub struct QueryRequest {
    query: String,

    #[serde(default)]
    params: HashMap<String, RequestParamValue>,

    #[serde(default)]
    files: Vec<RequestFileInput>,

    #[serde(default, rename = "elevatedPassword")]
    pub elevated_password: Option<String>,
}

#[derive(Serialize)]
pub struct QueryResponse {
    status: &'static str,
    function: String,
    result: OutputPayload,
}

pub async fn execute_steps(
    state: Arc<AppState>,
    steps: Vec<ExecutionStep>,
    auth: &ExecutionAuth,
) -> Result<OutputPayload> {
    let multiple = steps.len() > 1;
    let mut outputs = Vec::new();

    for step in steps {
        let result = match step {
            ExecutionStep::Http {
                method,
                url,
                headers,
                body,
                multipart_fields,
                directory,
            } => {
                info!("🌐 {} {}", method, url);
                debug!("HEADERS: {:?}", headers);
                if let Some(directory) = directory.as_ref() {
                    info!(" directory={}", directory);
                }
                if let Some(body) = body.as_ref() {
                    info!(" {}", body);
                } else if !multipart_fields.is_empty() {
                    info!(" multipart/form-data with {} parts", multipart_fields.len());
                }
                execute_http(
                    &state.http,
                    method,
                    url,
                    headers,
                    body,
                    multipart_fields,
                    directory,
                )
                .await
            }
            ExecutionStep::Command {
                program,
                args,
                directory,
                elevated,
            } => {
                info!(" {} {:?}", program, args);
                if let Some(directory) = directory.as_ref() {
                    info!(" directory={}", directory);
                }
                if elevated || program == "sudo" {
                    info!(" elevated=true");
                }
                execute_command(&state.http, program, args, directory, elevated, auth).await
            }
        }?;

        let payload = parse_output_payload(result.bytes);
        if !multiple {
            return Ok(payload);
        }
        outputs.push(serde_json::to_value(payload).map_err(|e| {
            AppError::InternalServerError(format!("Failed to serialize step output: {}", e))
        })?);
    }

    Ok(OutputPayload::Json(serde_json::Value::Array(outputs)))
}

#[cfg(feature = "ui")]
#[derive(Deserialize)]
pub struct LogsQuery {
    since: Option<u64>,
}

#[cfg(feature = "ui")]
#[derive(Serialize)]
pub struct UiLogResponse {
    next_cursor: u64,
    entries: Vec<UiLogLine>,
}

#[cfg(feature = "ui")]
#[derive(Serialize)]
pub struct UiLogLine {
    id: u64,
    line: String,
}

#[cfg(feature = "ui")]
#[derive(Deserialize)]
pub struct UploadQuery {
    filename: Option<String>,
    label: Option<String>,
    content_type: Option<String>,
}

#[cfg(feature = "ui")]
#[derive(Serialize)]
pub struct UploadResponse {
    path: String,
    filename: Option<String>,
    content_type: Option<String>,
    label: Option<String>,
}

#[cfg(feature = "ui")]
#[derive(Deserialize)]
pub struct ModelUploadQuery {
    target: String,
    filename: Option<String>,
}

#[cfg(feature = "ui")]
#[derive(Serialize)]
pub struct ModelUploadResponse {
    target: String,
    path: String,
    filename: String,
}

#[cfg(feature = "ui")]
#[derive(Serialize)]
pub struct SimpleStatusResponse {
    status: &'static str,
    message: String,
}

impl QueryRequest {
    fn has_explicit_param(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum RequestFileInput {
    Path(String),
    File(FileInput),
}

impl RequestFileInput {
    fn into_file_input(self) -> FileInput {
        match self {
            Self::Path(path) => FileInput {
                path,
                filename: None,
                content_type: None,
                label: None,
            },
            Self::File(file) => file,
        }
    }
}

fn coerce_explicit_param(param: &Param, value: RequestParamValue) -> Result<ParamValue> {
    match (&param.kind, value) {
        (ParamKind::Text, RequestParamValue::Text(text)) => Ok(ParamValue::Text(text)),
        (ParamKind::Text, RequestParamValue::TextList(values)) => {
            Ok(ParamValue::Text(values.join(",")))
        }
        (ParamKind::Text, RequestParamValue::File(_))
        | (ParamKind::Text, RequestParamValue::FileList(_)) => Err(AppError::BadRequest(format!(
            "Parameter '{}' expects text, not file input",
            param.name
        ))),

        (ParamKind::File, RequestParamValue::Text(path)) => Ok(ParamValue::File(FileInput {
            path,
            filename: None,
            content_type: None,
            label: None,
        })),
        (ParamKind::File, RequestParamValue::File(file)) => Ok(ParamValue::File(file)),
        (ParamKind::File, RequestParamValue::TextList(values)) => {
            if values.len() != 1 {
                return Err(AppError::BadRequest(format!(
                    "Parameter '{}' expects a single file",
                    param.name
                )));
            }
            Ok(ParamValue::File(FileInput {
                path: values[0].clone(),
                filename: None,
                content_type: None,
                label: None,
            }))
        }
        (ParamKind::File, RequestParamValue::FileList(files)) => {
            if files.len() != 1 {
                return Err(AppError::BadRequest(format!(
                    "Parameter '{}' expects a single file",
                    param.name
                )));
            }
            Ok(ParamValue::File(
                files.into_iter().next().expect("checked len"),
            ))
        }

        (ParamKind::Files, RequestParamValue::Text(path)) => {
            Ok(ParamValue::Files(vec![FileInput {
                path,
                filename: None,
                content_type: None,
                label: None,
            }]))
        }
        (ParamKind::Files, RequestParamValue::TextList(values)) => Ok(ParamValue::Files(
            values
                .into_iter()
                .map(|path| FileInput {
                    path,
                    filename: None,
                    content_type: None,
                    label: None,
                })
                .collect(),
        )),
        (ParamKind::Files, RequestParamValue::File(file)) => Ok(ParamValue::Files(vec![file])),
        (ParamKind::Files, RequestParamValue::FileList(files)) => Ok(ParamValue::Files(files)),
    }
}

fn resolve_params(
    req: &QueryRequest,
    func_params: &[Param],
    extracted: HashMap<String, String>,
) -> Result<ParamValues> {
    let mut resolved = ParamValues::new();

    for param in func_params {
        if let Some(explicit) = req.params.get(&param.name).cloned() {
            resolved.insert(param.name.clone(), coerce_explicit_param(param, explicit)?);
            continue;
        }

        if param.is_text() {
            if let Some(value) = extracted.get(&param.name) {
                resolved.insert(param.name.clone(), ParamValue::Text(value.clone()));
            }
        }
    }

    bind_uploaded_files(req, func_params, &mut resolved)?;

    Ok(resolved)
}

fn normalize_label(label: &str) -> String {
    label.trim().to_ascii_lowercase()
}

fn param_label_matches(param: &Param, label: &str) -> bool {
    let label = normalize_label(label);

    normalize_label(&param.name) == label
        || param.tag.as_deref().map(normalize_label).as_deref() == Some(label.as_str())
        || param
            .accept_labels
            .iter()
            .any(|accepted| normalize_label(accepted) == label)
}

fn bind_file_param(param: &Param, file: FileInput, resolved: &mut ParamValues) -> Result<bool> {
    match param.kind {
        ParamKind::File => {
            if resolved.contains_key(&param.name) {
                return Ok(false);
            }
            resolved.insert(param.name.clone(), ParamValue::File(file));
            Ok(true)
        }
        ParamKind::Files => {
            match resolved.get_mut(&param.name) {
                Some(ParamValue::Files(files)) => files.push(file),
                Some(_) => {
                    return Err(AppError::BadRequest(format!(
                        "Parameter '{}' has conflicting file bindings",
                        param.name
                    )))
                }
                None => {
                    resolved.insert(param.name.clone(), ParamValue::Files(vec![file]));
                }
            }
            Ok(true)
        }
        ParamKind::Text => Ok(false),
    }
}

fn bind_uploaded_files(
    req: &QueryRequest,
    func_params: &[Param],
    resolved: &mut ParamValues,
) -> Result<()> {
    let mut remaining: Vec<FileInput> = req
        .files
        .clone()
        .into_iter()
        .map(RequestFileInput::into_file_input)
        .collect();

    let mut unmatched = Vec::new();

    for file in remaining.drain(..) {
        let Some(label) = file.label.as_deref() else {
            unmatched.push(file);
            continue;
        };

        let mut matched = false;
        for param in func_params
            .iter()
            .filter(|param| param.kind != ParamKind::Text && !req.has_explicit_param(&param.name))
        {
            if param_label_matches(param, label) && bind_file_param(param, file.clone(), resolved)?
            {
                matched = true;
                break;
            }
        }

        if !matched {
            unmatched.push(file);
        }
    }

    let file_params: Vec<&Param> = func_params
        .iter()
        .filter(|param| param.kind != ParamKind::Text && !req.has_explicit_param(&param.name))
        .collect();

    let last_multi_file_param = file_params
        .iter()
        .rposition(|param| param.kind == ParamKind::Files);

    let mut remaining_iter = unmatched.into_iter().peekable();

    for (index, param) in file_params.iter().enumerate() {
        if resolved.contains_key(&param.name) && param.kind == ParamKind::File {
            continue;
        }

        match param.kind {
            ParamKind::Text => {}
            ParamKind::File => {
                let Some(file) = remaining_iter.next() else {
                    continue;
                };
                resolved.insert(param.name.clone(), ParamValue::File(file));
            }
            ParamKind::Files => {
                let files: Vec<FileInput> = if Some(index) == last_multi_file_param {
                    remaining_iter.by_ref().collect()
                } else {
                    match remaining_iter.next() {
                        Some(file) => vec![file],
                        None => Vec::new(),
                    }
                };

                if !files.is_empty() {
                    match resolved.get_mut(&param.name) {
                        Some(ParamValue::Files(existing)) => existing.extend(files),
                        Some(_) => {
                            return Err(AppError::BadRequest(format!(
                                "Parameter '{}' has conflicting file bindings",
                                param.name
                            )))
                        }
                        None => {
                            resolved.insert(param.name.clone(), ParamValue::Files(files));
                        }
                    }
                }
            }
        }
    }

    if remaining_iter.peek().is_some() {
        return Err(AppError::BadRequest(
            "Too many uploaded files were provided for the selected function".to_string(),
        ));
    }

    Ok(())
}

fn text_params_to_extract(req: &QueryRequest, func_params: &[Param]) -> Vec<Param> {
    func_params
        .iter()
        .filter(|param| param.is_text() && !req.has_explicit_param(&param.name))
        .cloned()
        .collect()
}

fn missing_required_params(func_params: &[Param], params: &ParamValues) -> Vec<String> {
    func_params
        .iter()
        .filter(|param| param.required)
        .filter(|param| match params.get(&param.name) {
            Some(value) => value.is_missing(),
            None => true,
        })
        .map(|param| param.name.clone())
        .collect()
}

fn decode_file_input(value: serde_json::Value) -> Option<FileInput> {
    match value {
        serde_json::Value::String(path) => Some(FileInput {
            path,
            filename: None,
            content_type: None,
            label: None,
        }),
        other => serde_json::from_value(other).ok(),
    }
}

fn parse_output_payload(bytes: Vec<u8>) -> OutputPayload {
    if bytes.is_empty() {
        return OutputPayload::Empty;
    }

    match String::from_utf8(bytes) {
        Ok(text) => {
            if text.trim().is_empty() {
                return OutputPayload::Empty;
            }

            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(file) = decode_file_input(value.clone()) {
                    return OutputPayload::File(file);
                }

                if let serde_json::Value::Array(items) = &value {
                    let files: Option<Vec<FileInput>> =
                        items.iter().cloned().map(decode_file_input).collect();
                    if let Some(files) = files {
                        return OutputPayload::Files(files);
                    }
                }

                return OutputPayload::Json(value);
            }

            OutputPayload::String(text)
        }
        Err(err) => OutputPayload::Binary(STANDARD.encode(err.into_bytes())),
    }
}

pub async fn execute_query_payload(
    state: Arc<AppState>,
    req: QueryRequest,
) -> Result<QueryResponse> {
    info!("\n🔍 {}", req.query);
    let func = match route(req.query.clone(), state.clone()).await? {
        Some(f) => f,
        None => return Err(AppError::NotFound("No function matched".to_string())),
    };
    info!(" Function: {}", func.name);
    let extractable_params = text_params_to_extract(&req, &func.parameters);
    let extracted = if extractable_params.is_empty() {
        Ok(HashMap::new())
    } else {
        extract_params(
            req.query.clone(),
            extractable_params,
            state.gliner_model().await,
        )
        .await
    }?;
    let params = resolve_params(&req, &func.parameters, extracted)?;
    debug!(" Params: {:?}", params);
    let missing = missing_required_params(&func.parameters, &params);
    if !missing.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Missing required parameters: {}",
            missing.join(", ")
        )));
    }
    let auth = ExecutionAuth {
        elevated_password: req.elevated_password.clone(),
    };
    let result = match build_execution_plan(&func, &params)? {
        ExecutionPlan::Steps(steps) => execute_steps(state.clone(), steps, &auth).await,
        ExecutionPlan::NoExecution {
            function_name,
            parameters,
        } => {
            let mut params_list = Vec::new();
            for (name, value) in parameters.iter() {
                params_list.push(format!("{}:{:?}", name, value));
            }

            let mut params_json = serde_json::Map::new();
            params_json.insert(
                "function_name".to_string(),
                serde_json::Value::String(function_name.clone()),
            );
            params_json.insert(
                "parameters".to_string(),
                serde_json::Value::Array(
                    params_list
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );

            return Ok(QueryResponse {
                status: "success",
                function: function_name,
                result: OutputPayload::Json(serde_json::Value::Object(params_json)),
            });
        }
    }?;

    Ok(QueryResponse {
        status: "success",
        function: func.name,
        result,
    })
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>> {
    let started = Instant::now();
    match execute_query_payload(state, req).await {
        Ok(response) => {
            info!(
                "⏱️ status=200 latency_ms={} function={}",
                started.elapsed().as_millis(),
                response.function
            );
            Ok(Json(response))
        }
        Err(err) => {
            info!(
                "⏱️ status={} latency_ms={} error={}",
                err.status_code().as_u16(),
                started.elapsed().as_millis(),
                err
            );
            Err(err)
        }
    }
}

#[cfg(feature = "ui")]
pub async fn logs_handler(Query(query): Query<LogsQuery>) -> Result<Json<UiLogResponse>> {
    let (next_cursor, entries) = crate::ui_logs::read_logs(query.since);
    Ok(Json(UiLogResponse {
        next_cursor,
        entries: entries
            .into_iter()
            .map(|entry| UiLogLine {
                id: entry.id,
                line: entry.line,
            })
            .collect(),
    }))
}

#[cfg(feature = "ui")]
pub async fn upload_ui_file(
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<Json<UploadResponse>> {
    let filename = query
        .filename
        .as_deref()
        .map(sanitize_filename)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "upload.bin".to_string());
    let content_type = query.content_type.clone();

    let upload_dir = std::env::temp_dir().join("quicktasks-ui-uploads");
    tokio::fs::create_dir_all(&upload_dir).await.map_err(|e| {
        AppError::InternalServerError(format!("Failed to create upload directory: {}", e))
    })?;

    let unique_name = format!("{}-{}", unique_upload_prefix(), filename);
    let path = upload_dir.join(unique_name);
    tokio::fs::write(&path, &body).await.map_err(|e| {
        AppError::InternalServerError(format!("Failed to save uploaded file: {}", e))
    })?;

    Ok(Json(UploadResponse {
        path: path.to_string_lossy().to_string(),
        filename: Some(filename),
        content_type,
        label: query.label.filter(|value| !value.trim().is_empty()),
    }))
}

#[cfg(feature = "ui")]
pub async fn upload_model_file(
    Query(query): Query<ModelUploadQuery>,
    request: Request,
) -> Result<Json<ModelUploadResponse>> {
    let paths = model_asset_paths();
    let path = match query.target.as_str() {
        "gte_model" => paths.gte_model.as_str(),
        "gte_tokenizer" => paths.gte_tokenizer.as_str(),
        "gliner_model" => paths.gliner_model.as_str(),
        "gliner_tokenizer" => paths.gliner_tokenizer.as_str(),
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown model upload target: {}",
                other
            )))
        }
    };

    let filename = query
        .filename
        .as_deref()
        .map(sanitize_filename)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "uploaded-file".to_string());

    write_request_body_to_path(request, Path::new(path)).await?;

    info!(" Replaced {}", query.target);

    Ok(Json(ModelUploadResponse {
        target: query.target,
        path: path.to_string(),
        filename,
    }))
}

#[cfg(feature = "ui")]
async fn write_request_body_to_path(request: Request, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to create model directory: {}", e))
        })?;
    }

    let temp_path = path.with_extension("uploading");
    let mut output = tokio::fs::File::create(&temp_path).await.map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to create temporary upload file '{}': {}",
            temp_path.display(),
            e
        ))
    })?;

    let mut stream = request.into_body().into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            AppError::InternalServerError(format!("Failed to read upload stream: {}", e))
        })?;
        output.write_all(&chunk).await.map_err(|e| {
            AppError::InternalServerError(format!(
                "Failed to write uploaded file '{}': {}",
                temp_path.display(),
                e
            ))
        })?;
    }

    output.flush().await.map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to flush uploaded file '{}': {}",
            temp_path.display(),
            e
        ))
    })?;

    drop(output);

    tokio::fs::rename(&temp_path, path).await.map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to replace model file '{}': {}",
            path.display(),
            e
        ))
    })?;

    Ok(())
}

#[cfg(feature = "ui")]
pub async fn reload_models_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SimpleStatusResponse>> {
    state.reload_models().await?;
    Ok(Json(SimpleStatusResponse {
        status: "success",
        message: "Models reloaded successfully".to_string(),
    }))
}

#[cfg(feature = "ui")]
fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

#[cfg(feature = "ui")]
fn unique_upload_prefix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    nanos.to_string()
}
