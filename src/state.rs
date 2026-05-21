use crate::error::{AppError, Result};
use crate::models::Components;
use crate::models::{Node, RootJson};
use gliner::model::{params::Parameters as GlinerParams, pipeline::span::SpanMode, GLiNER};
use gte::{params::Parameters as GteParams, rerank::pipeline::RerankingPipeline};
use orp::model::Model;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::RwLock as StdRwLock;
use tokio::sync::RwLock;
use tracing::info;

const DEFAULT_GTE_MODEL_URL: &str =
    "https://huggingface.co/Alibaba-NLP/gte-reranker-modernbert-base/resolve/main/onnx/model.onnx";
const DEFAULT_GTE_TOKENIZER_URL: &str =
    "https://huggingface.co/Alibaba-NLP/gte-modernbert-base/resolve/main/tokenizer.json";
const DEFAULT_GLINER_MODEL_URL: &str =
    "https://huggingface.co/onnx-community/gliner_small-v2.1/resolve/main/onnx/model.onnx";
const DEFAULT_GLINER_TOKENIZER_URL: &str =
    "https://huggingface.co/onnx-community/gliner_small-v2.1/resolve/main/tokenizer.json";
const DEFAULT_CONFIG_PATH: &str = "data.json";
const GTE_MODEL_RELATIVE_PATH: &str = "crossencoder/model.onnx";
const GTE_TOKENIZER_RELATIVE_PATH: &str = "crossencoder/tokenizer.json";
const GLINER_MODEL_RELATIVE_PATH: &str = "ner/model.onnx";
const GLINER_TOKENIZER_RELATIVE_PATH: &str = "ner/tokenizer.json";

static RUNTIME_PATHS: OnceLock<RuntimePaths> = OnceLock::new();

struct RuntimePaths {
    model_dir: PathBuf,
    bundled_model_dir: Option<PathBuf>,
    model_path_settings: PathBuf,
    overrides: StdRwLock<ModelPathOverrides>,
}

#[derive(Clone)]
pub struct ModelAssetPaths {
    pub gte_model: String,
    pub gte_tokenizer: String,
    pub gliner_model: String,
    pub gliner_tokenizer: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct ModelPathOverrides {
    gte_model: Option<String>,
    gte_tokenizer: Option<String>,
    gliner_model: Option<String>,
    gliner_tokenizer: Option<String>,
}

pub fn configure_runtime_paths(app_data_dir: PathBuf, resource_dir: Option<PathBuf>) -> Result<()> {
    let model_dir = env_path("QUICKTASKS_MODEL_DIR")
        .or_else(|| env_path("EFFICEINTNLP_MODEL_DIR"))
        .unwrap_or_else(|| app_data_dir.join("models"));

    fs::create_dir_all(&model_dir).map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to create model directory '{}': {}",
            model_dir.display(),
            e
        ))
    })?;

    let model_path_settings = app_data_dir.join("model-paths.json");
    let overrides = load_model_path_overrides(&model_path_settings)?;

    let bundled_model_dir = env_path("QUICKTASKS_BUNDLED_MODEL_DIR").or_else(|| {
        resource_dir
            .as_ref()
            .map(|dir| dir.join("models"))
            .filter(|dir| dir.exists())
    });

    let _ = RUNTIME_PATHS.set(RuntimePaths {
        model_dir,
        bundled_model_dir,
        model_path_settings,
        overrides: StdRwLock::new(overrides),
    });

    Ok(())
}

pub fn config_path() -> String {
    std::env::var("QUICKTASKS_CONFIG")
        .or_else(|_| std::env::var("EFFICEINTNLP_CONFIG"))
        .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string())
}

pub fn model_asset_paths() -> ModelAssetPaths {
    let model_dir = runtime_model_dir();
    let overrides = runtime_model_overrides();

    let default_path = |relative: &str| -> String {
        PathBuf::from(&model_dir)
            .join(relative)
            .to_string_lossy()
            .to_string()
    };

    ModelAssetPaths {
        gte_model: overrides
            .gte_model
            .or_else(|| env_string("QUICKTASKS_GTE_MODEL_PATH"))
            .or_else(|| env_string("EFFICEINTNLP_GTE_MODEL_PATH"))
            .unwrap_or_else(|| default_path(GTE_MODEL_RELATIVE_PATH)),
        gte_tokenizer: overrides
            .gte_tokenizer
            .or_else(|| env_string("QUICKTASKS_GTE_TOKENIZER_PATH"))
            .or_else(|| env_string("EFFICEINTNLP_GTE_TOKENIZER_PATH"))
            .unwrap_or_else(|| default_path(GTE_TOKENIZER_RELATIVE_PATH)),
        gliner_model: overrides
            .gliner_model
            .or_else(|| env_string("QUICKTASKS_GLINER_MODEL_PATH"))
            .or_else(|| env_string("EFFICEINTNLP_GLINER_MODEL_PATH"))
            .unwrap_or_else(|| default_path(GLINER_MODEL_RELATIVE_PATH)),
        gliner_tokenizer: overrides
            .gliner_tokenizer
            .or_else(|| env_string("QUICKTASKS_GLINER_TOKENIZER_PATH"))
            .or_else(|| env_string("EFFICEINTNLP_GLINER_TOKENIZER_PATH"))
            .unwrap_or_else(|| default_path(GLINER_TOKENIZER_RELATIVE_PATH)),
    }
}

pub fn set_model_asset_path(target: &str, path: PathBuf) -> Result<String> {
    let metadata = fs::metadata(&path).map_err(|e| {
        AppError::BadRequest(format!(
            "Failed to stat model asset path '{}': {}",
            path.display(),
            e
        ))
    })?;

    if !metadata.is_file() {
        return Err(AppError::BadRequest(format!(
            "Model asset path '{}' is not a file",
            path.display()
        )));
    }

    let normalized = path.canonicalize().map_err(|e| {
        AppError::BadRequest(format!(
            "Failed to resolve model asset path '{}': {}",
            path.display(),
            e
        ))
    })?;
    let normalized = normalized.to_string_lossy().to_string();

    let runtime_paths = RUNTIME_PATHS.get().ok_or_else(|| {
        AppError::InternalServerError("Runtime paths are not configured".to_string())
    })?;
    let mut overrides = runtime_paths.overrides.write().map_err(|_| {
        AppError::InternalServerError("Failed to lock model path overrides".to_string())
    })?;

    match target {
        "gte_model" => overrides.gte_model = Some(normalized.clone()),
        "gte_tokenizer" => overrides.gte_tokenizer = Some(normalized.clone()),
        "gliner_model" => overrides.gliner_model = Some(normalized.clone()),
        "gliner_tokenizer" => overrides.gliner_tokenizer = Some(normalized.clone()),
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown model path target: {}",
                other
            )))
        }
    }

    save_model_path_overrides(&runtime_paths.model_path_settings, &overrides)?;

    Ok(normalized)
}

fn runtime_model_dir() -> String {
    env_string("QUICKTASKS_MODEL_DIR")
        .or_else(|| env_string("EFFICEINTNLP_MODEL_DIR"))
        .or_else(|| {
            RUNTIME_PATHS
                .get()
                .map(|paths| paths.model_dir.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "models".to_string())
}

fn runtime_model_overrides() -> ModelPathOverrides {
    RUNTIME_PATHS
        .get()
        .and_then(|paths| {
            paths
                .overrides
                .read()
                .ok()
                .map(|overrides| overrides.clone())
        })
        .map(existing_model_path_overrides)
        .unwrap_or_default()
}

fn existing_model_path_overrides(mut overrides: ModelPathOverrides) -> ModelPathOverrides {
    let keep_existing = |value: Option<String>| -> Option<String> {
        value.filter(|path| Path::new(path).is_file())
    };

    overrides.gte_model = keep_existing(overrides.gte_model);
    overrides.gte_tokenizer = keep_existing(overrides.gte_tokenizer);
    overrides.gliner_model = keep_existing(overrides.gliner_model);
    overrides.gliner_tokenizer = keep_existing(overrides.gliner_tokenizer);
    overrides
}

fn load_model_path_overrides(path: &Path) -> Result<ModelPathOverrides> {
    if !path.exists() {
        return Ok(ModelPathOverrides::default());
    }

    let raw = fs::read_to_string(path).map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to read model path settings '{}': {}",
            path.display(),
            e
        ))
    })?;

    serde_json::from_str(&raw).map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to parse model path settings '{}': {}",
            path.display(),
            e
        ))
    })
}

fn save_model_path_overrides(path: &Path, overrides: &ModelPathOverrides) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            AppError::InternalServerError(format!(
                "Failed to create model path settings directory '{}': {}",
                parent.display(),
                e
            ))
        })?;
    }

    let raw = serde_json::to_string_pretty(overrides).map_err(|e| {
        AppError::InternalServerError(format!("Failed to serialize model path settings: {}", e))
    })?;

    fs::write(path, raw).map_err(|e| {
        AppError::InternalServerError(format!(
            "Failed to write model path settings '{}': {}",
            path.display(),
            e
        ))
    })
}

fn bundled_model_asset(relative_path: &str) -> Option<PathBuf> {
    RUNTIME_PATHS
        .get()
        .and_then(|paths| paths.bundled_model_dir.as_ref())
        .map(|dir| dir.join(relative_path))
        .filter(|path| path.exists())
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env_string(name).map(PathBuf::from)
}

pub struct ModelResources {
    pub router_model: Arc<Model>,
    pub router_pipeline: Arc<RerankingPipeline>,
    pub router_params: Arc<GteParams>,
    pub gliner: Arc<GLiNER<SpanMode>>,
}

#[derive(Clone)]
pub struct RoutingResources {
    pub router_model: Arc<Model>,
    pub router_pipeline: Arc<RerankingPipeline>,
    pub router_params: Arc<GteParams>,
}

pub struct AppState {
    pub resources: RwLock<ModelResources>,
    pub root: RwLock<Node>,
    pub components: RwLock<Option<Components>>,
    pub http: Client,
}

impl AppState {
    fn load_config() -> Result<RootJson> {
        let config_path = config_path();
        let raw = fs::read_to_string(&config_path).map_err(|e| {
            AppError::InternalServerError(format!("Failed to read config '{}': {}", config_path, e))
        })?;
        serde_json::from_str(&raw).map_err(|e| {
            AppError::InternalServerError(format!(
                "Failed to parse config '{}': {}",
                config_path, e
            ))
        })
    }

    pub async fn new() -> Result<Self> {
        let parsed = Self::load_config()?;
        Self::from_config(parsed).await
    }

    pub async fn from_config(parsed: RootJson) -> Result<Self> {
        info!(" Loading models at startup");
        let resources = Self::load_model_resources(parsed.components.as_ref()).await?;
        info!(" Models ready");

        Ok(Self {
            resources: RwLock::new(resources),
            root: RwLock::new(parsed.architecture),
            components: RwLock::new(parsed.components),
            http: Client::new(),
        })
    }

    pub async fn update_agent_config(&self, new_config: RootJson) -> Result<()> {
        let current_components = self.components.read().await.clone();
        let should_reload_models = current_components != new_config.components;

        if should_reload_models {
            info!("🔁 Reloading models for agent change");
            let resources = Self::load_model_resources(new_config.components.as_ref()).await?;
            let mut resources_lock = self.resources.write().await;
            *resources_lock = resources;
            drop(resources_lock);
        } else {
            info!(" Applying agent config without model reload");
        }

        let mut root_lock = self.root.write().await;
        *root_lock = new_config.architecture;
        drop(root_lock);

        let mut components_lock = self.components.write().await;
        *components_lock = new_config.components;
        info!(" Agent config applied");
        Ok(())
    }

    pub async fn routing_resources(&self) -> RoutingResources {
        let resources = self.resources.read().await;
        RoutingResources {
            router_model: resources.router_model.clone(),
            router_pipeline: resources.router_pipeline.clone(),
            router_params: resources.router_params.clone(),
        }
    }

    pub async fn gliner_model(&self) -> Arc<GLiNER<SpanMode>> {
        self.resources.read().await.gliner.clone()
    }

    pub async fn reload_models(&self) -> Result<()> {
        info!("🔁 Reloading models from current files");
        let components = self.components.read().await.clone();
        let resources = Self::load_model_resources(components.as_ref()).await?;
        let mut resources_lock = self.resources.write().await;
        *resources_lock = resources;
        info!(" Models reloaded");
        Ok(())
    }

    async fn load_model_resources(components: Option<&Components>) -> Result<ModelResources> {
        let paths = model_asset_paths();
        let gte_model_url = components
            .and_then(|c| c.function_mapper.as_ref())
            .and_then(|m| m.model_url.clone())
            .unwrap_or_else(|| DEFAULT_GTE_MODEL_URL.into());

        let gte_tokenizer_url = components
            .and_then(|c| c.function_mapper.as_ref())
            .and_then(|m| m.tokenizer_url.clone())
            .unwrap_or_else(|| DEFAULT_GTE_TOKENIZER_URL.into());

        let gliner_model_url = components
            .and_then(|c| c.entity_recognizer.as_ref())
            .and_then(|m| m.model_url.clone())
            .unwrap_or_else(|| DEFAULT_GLINER_MODEL_URL.into());

        let gliner_tokenizer_url = components
            .and_then(|c| c.entity_recognizer.as_ref())
            .and_then(|m| m.tokenizer_url.clone())
            .unwrap_or_else(|| DEFAULT_GLINER_TOKENIZER_URL.into());

        Self::prepare_model_asset(
            &gte_model_url,
            &paths.gte_model,
            GTE_MODEL_RELATIVE_PATH,
            DEFAULT_GTE_MODEL_URL,
        )
        .await?;
        Self::prepare_model_asset(
            &gte_tokenizer_url,
            &paths.gte_tokenizer,
            GTE_TOKENIZER_RELATIVE_PATH,
            DEFAULT_GTE_TOKENIZER_URL,
        )
        .await?;
        Self::prepare_model_asset(
            &gliner_model_url,
            &paths.gliner_model,
            GLINER_MODEL_RELATIVE_PATH,
            DEFAULT_GLINER_MODEL_URL,
        )
        .await?;
        Self::prepare_model_asset(
            &gliner_tokenizer_url,
            &paths.gliner_tokenizer,
            GLINER_TOKENIZER_RELATIVE_PATH,
            DEFAULT_GLINER_TOKENIZER_URL,
        )
        .await?;

        let gte_model_path = paths.gte_model.clone();
        let gte_tokenizer_path = paths.gte_tokenizer.clone();
        let gliner_model_path = paths.gliner_model.clone();
        let gliner_tokenizer_path = paths.gliner_tokenizer.clone();

        tokio::task::spawn_blocking(move || {
            let params = Arc::new(GteParams::default().with_sigmoid(true));

            let pipeline = RerankingPipeline::new(&gte_tokenizer_path, &params).map_err(|e| {
                AppError::InternalServerError(format!("Failed to create RerankingPipeline: {}", e))
            })?;

            let model = Model::new(&gte_model_path, Default::default()).map_err(|e| {
                AppError::InternalServerError(format!("Failed to create GTE Model: {}", e))
            })?;

            let gliner = GLiNER::<SpanMode>::new(
                GlinerParams::default(),
                Default::default(),
                &gliner_tokenizer_path,
                &gliner_model_path,
            )
            .map_err(|e| {
                AppError::InternalServerError(format!("Failed to create GLiNER model: {}", e))
            })?;

            Ok(ModelResources {
                router_model: Arc::new(model),
                router_pipeline: Arc::new(pipeline),
                router_params: params,
                gliner: Arc::new(gliner),
            })
        })
        .await
        .map_err(|e| {
            AppError::InternalServerError(format!("Task join error while loading models: {}", e))
        })?
    }

    async fn prepare_model_asset(
        source: &str,
        destination: &str,
        relative_path: &str,
        default_source: &str,
    ) -> Result<()> {
        if Path::new(destination).exists() {
            info!(" Cached model asset: {}", destination);
            return Ok(());
        }

        if source.starts_with("http://") || source.starts_with("https://") {
            if source == default_source {
                if let Some(bundled_path) = bundled_model_asset(relative_path) {
                    Self::copy_model_asset(&bundled_path, Path::new(destination)).await?;
                    info!(
                        " Seeded model asset from bundled resource {}",
                        bundled_path.display()
                    );
                    return Ok(());
                }
            }

            crate::downloader::download_if_missing(source, destination).await
        } else if !source.trim().is_empty() {
            let source_path = Self::resolve_model_source(source, relative_path)?;
            if source_path != Path::new(destination) {
                Self::copy_model_asset(&source_path, Path::new(destination)).await?;
                info!(" Updated model asset {}", destination);
            }
            Ok(())
        } else {
            Err(AppError::BadRequest(
                "Model asset source cannot be empty".to_string(),
            ))
        }
    }

    fn resolve_model_source(source: &str, default_relative_path: &str) -> Result<PathBuf> {
        let source_path = PathBuf::from(source);
        if source_path.exists() {
            return Ok(source_path);
        }

        if source_path.is_relative() {
            if let Some(bundled_path) = bundled_model_asset(source) {
                return Ok(bundled_path);
            }

            if let Some(stripped) = source.strip_prefix("models/") {
                if let Some(bundled_path) = bundled_model_asset(stripped) {
                    return Ok(bundled_path);
                }
            }

            if source == default_relative_path {
                if let Some(bundled_path) = bundled_model_asset(default_relative_path) {
                    return Ok(bundled_path);
                }
            }

            if let Some(paths) = RUNTIME_PATHS.get() {
                let app_data_path = paths.model_dir.join(source);
                if app_data_path.exists() {
                    return Ok(app_data_path);
                }

                if let Some(stripped) = source.strip_prefix("models/") {
                    let app_data_path = paths.model_dir.join(stripped);
                    if app_data_path.exists() {
                        return Ok(app_data_path);
                    }
                }
            }
        }

        Err(AppError::BadRequest(format!(
            "Model asset source '{}' was not found. Bundle it under 'models/{}', provide an absolute path, or use an http(s) URL.",
            source,
            default_relative_path
        )))
    }

    async fn copy_model_asset(source: &Path, destination: &Path) -> Result<()> {
        if source == destination {
            return Ok(());
        }

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::InternalServerError(format!("Failed to create model directory: {}", e))
            })?;
        }

        let temp_path = destination.with_extension("copying");
        tokio::fs::copy(source, &temp_path).await.map_err(|e| {
            AppError::InternalServerError(format!(
                "Failed to copy model asset '{}' to '{}': {}",
                source.display(),
                temp_path.display(),
                e
            ))
        })?;

        tokio::fs::rename(&temp_path, destination)
            .await
            .map_err(|e| {
                AppError::InternalServerError(format!(
                    "Failed to move model asset '{}' to '{}': {}",
                    temp_path.display(),
                    destination.display(),
                    e
                ))
            })?;

        Ok(())
    }
}
