use crate::error::{AppError, Result};
use std::{fs, path::Path};
use tracing::info;

pub async fn download_if_missing(url: &str, path: &str) -> Result<()> {
    if Path::new(path).exists() {
        info!("⚡ Cached: {}", path);
        return Ok(());
    }

    info!("⬇️ Downloading {}", url);

    let bytes =
        reqwest::get(url).await?.bytes().await.map_err(|e| {
            AppError::InternalServerError(format!("Failed to download bytes: {}", e))
        })?;

    let parent_dir = Path::new(path).parent().ok_or_else(|| {
        AppError::InternalServerError(format!(
            "Could not determine parent directory for path: {}",
            path
        ))
    })?;
    fs::create_dir_all(parent_dir)
        .map_err(|e| AppError::InternalServerError(format!("Failed to create directory: {}", e)))?;
    fs::write(path, &bytes)
        .map_err(|e| AppError::InternalServerError(format!("Failed to write file: {}", e)))?;

    info!("✅ Saved {}", path);
    Ok(())
}
