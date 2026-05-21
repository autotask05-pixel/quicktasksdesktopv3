use crate::error::{AppError, Result};
use crate::models::Node;
use crate::state::{AppState, RoutingResources};
use gte::rerank::input::TextInput as GteInput;
use std::cmp::Ordering;
use std::sync::Arc;

fn best_child<'a>(
    query: &str,
    node: &'a Node,
    resources: &RoutingResources,
) -> Result<Option<&'a Node>> {
    if node.children.is_empty() {
        return Ok(None);
    }

    let candidates: Vec<String> = node
        .children
        .iter()
        .map(|child| format!("{} {}", child.name, child.desc))
        .collect();

    let pairs: Vec<(&str, &str)> = candidates
        .iter()
        .map(|candidate| (query, candidate.as_str()))
        .collect();
    let inputs = GteInput::from_str(&pairs);

    let outputs = resources
        .router_model
        .inference(
            inputs,
            &*resources.router_pipeline,
            &resources.router_params,
        )
        .map_err(|e| {
            AppError::InternalServerError(format!("Failed to run cross-encoder inference: {}", e))
        })?;

    let (index, _) = outputs
        .scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Ordering::Equal))
        .ok_or_else(|| AppError::InternalServerError("No scores found for routing".to_string()))?;

    Ok(node.children.get(index))
}

pub async fn route(query: String, state: Arc<AppState>) -> Result<Option<Node>> {
    let root_guard = state.root.read().await;
    let root_node = root_guard.clone();
    drop(root_guard);
    let resources = state.routing_resources().await;

    tokio::task::spawn_blocking(move || {
        fn recurse(query: &str, node: &Node, resources: &RoutingResources) -> Result<Option<Node>> {
            if node.children.is_empty() || node.node_type == "func" {
                return Ok(Some(node.clone()));
            }

            let Some(selected) = best_child(query, node, resources)? else {
                return Ok(None);
            };

            match selected.node_type.as_str() {
                "group" | "agent" => recurse(query, selected, resources),
                "func" => Ok(Some(selected.clone())),
                _ => Ok(None),
            }
        }

        recurse(&query, &root_node, &resources)
    })
    .await
    .map_err(|e| AppError::InternalServerError(format!("Task join error in routing: {}", e)))?
}
