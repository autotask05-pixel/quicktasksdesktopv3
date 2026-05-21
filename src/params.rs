use crate::error::{AppError, Result};
use crate::models::Param;
use gliner::model::{input::text::TextInput as GlinerInput, pipeline::span::SpanMode, GLiNER};
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;

fn extract_with_regex(text: &str, param: &Param) -> Result<Option<String>> {
    let Some(pattern) = &param.pattern else {
        return Ok(None);
    };

    let regex = Regex::new(pattern).map_err(|e| {
        AppError::BadRequest(format!(
            "Invalid regex for parameter '{}': {}",
            param.name, e
        ))
    })?;

    Ok(regex
        .captures(text)
        .and_then(|captures| captures.get(1).or_else(|| captures.get(0)))
        .map(|value| value.as_str().trim().to_string())
        .filter(|value| !value.is_empty()))
}

pub async fn extract_params(
    text: String,
    params: Vec<Param>,
    model: Arc<GLiNER<SpanMode>>,
) -> Result<HashMap<String, String>> {
    tokio::task::spawn_blocking(move || {
        let mut extracted = HashMap::new();
        let mut gliner_labels = Vec::new();
        let mut label_to_name = HashMap::new();

        for param in &params {
            let extractor = param.extractor.as_deref().unwrap_or("gliner");
            if extractor.eq_ignore_ascii_case("regex") {
                if let Some(value) = extract_with_regex(&text, param)? {
                    extracted.insert(param.name.clone(), value);
                }
                continue;
            }

            let label = param.tag.clone().unwrap_or_else(|| param.name.clone());
            label_to_name.insert(label.clone(), param.name.clone());
            gliner_labels.push(label);
        }

        if gliner_labels.is_empty() {
            return Ok(extracted);
        }

        let refs: Vec<&str> = gliner_labels.iter().map(|s| s.as_str()).collect();

        let input = GlinerInput::from_str(&[&text], &refs).map_err(|e| {
            AppError::InternalServerError(format!("Failed to create GlinerInput: {}", e))
        })?;
        let output = model.inference(input).map_err(|e| {
            AppError::InternalServerError(format!("Failed to run Gliner inference: {}", e))
        })?;

        if let Some(spans) = output.spans.into_iter().next() {
            for s in spans {
                let label = s.class().to_string();
                let key = label_to_name.get(&label).cloned().unwrap_or(label);
                extracted.entry(key).or_insert_with(|| s.text().to_string());
            }
        }

        Ok(extracted)
    })
    .await
    .map_err(|e| AppError::InternalServerError(format!("Task join error: {}", e)))?
}
