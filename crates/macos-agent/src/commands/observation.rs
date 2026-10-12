use serde::Serialize;
use serde_json::{Value, json};

/// AX bounds and cached/fresh trees are not raster visibility evidence.
pub const VISIBILITY_NOTICE: &str = "Accessibility metadata is visibility-unverified: controls may be occluded by an overlay, and AX metadata is not an ROI-visible-content boundary. Pixels and AX reads are not atomic. Verify a bounded visual postcondition before acting.";

#[derive(Debug, Clone, Serialize)]
pub struct ObservationScope {
    pub ax_visibility: &'static str,
    pub roi_requested: bool,
    pub notice: &'static str,
}

impl ObservationScope {
    pub fn new(roi_requested: bool) -> Self {
        Self {
            ax_visibility: "unverified",
            roi_requested,
            notice: VISIBILITY_NOTICE,
        }
    }
}

pub fn is_observation(command: &str) -> bool {
    matches!(command, "see" | "inspect_ui")
}

/// Qualify every returned AX list without guessing which controls are hidden.
/// Preserve IDs and bounds for AX targeting, and never certify upstream visibility claims.
pub fn qualify_ax_lists(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(elements)) = map.get_mut("ui_elements") {
                for element in elements {
                    if let Some(object) = element.as_object_mut() {
                        object.insert("adapter_visibility".into(), json!("unverified"));
                    }
                }
            }
            for child in map.values_mut() {
                qualify_ax_lists(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                qualify_ax_lists(child);
            }
        }
        _ => {}
    }
}

pub fn qualify_mcp_result(
    response: &mut Value,
    roi_requested: bool,
) -> Result<(), crate::error::CliError> {
    let Some(value) = response.get_mut("result") else {
        return Ok(()); // A JSON-RPC error carries no observation payload.
    };
    let result = value.as_object_mut().ok_or_else(|| {
        crate::error::CliError::upstream(
            "Peekaboo MCP emitted an invalid observation result: expected an object",
        )
        .with_operation("mcp")
    })?;
    let metadata = result.entry("_meta").or_insert_with(|| json!({}));
    if !metadata.is_object() {
        *metadata = json!({});
    }
    if let Some(metadata) = metadata.as_object_mut() {
        metadata.insert(
            "macos-agent/observation_scope".into(),
            json!(ObservationScope::new(roi_requested)),
        );
    }
    if let Some(structured) = result.get_mut("structuredContent") {
        qualify_ax_lists(structured);
    }
    let content = result.entry("content").or_insert_with(|| json!([]));
    if !content.is_array() {
        *content = json!([]);
    }
    if let Some(content) = content.as_array_mut() {
        // Peekaboo returns both human-readable and JSON-in-text tool content.
        for item in content.iter_mut() {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    if let Ok(mut parsed) = serde_json::from_str::<Value>(text) {
                        qualify_ax_lists(&mut parsed);
                        item["text"] = Value::String(parsed.to_string());
                    }
                }
            }
        }
        content.insert(0, json!({"type":"text", "text":VISIBILITY_NOTICE}));
    }
    Ok(())
}
