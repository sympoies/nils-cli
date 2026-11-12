//! Metadata-only projection before proxy observations cross the worker queue.
use super::*;

pub(super) fn server_observation(value: &Value) -> ServerProjection {
    if let Some(id) = value.get("id").filter(|id| json_id_key(id).is_some())
        && value.get("error").is_some()
    {
        return ServerProjection::Unique(json!({"id": id, "error": {}}));
    }
    if let (Some(id), Some(turn_id)) = (
        value.get("id"),
        value.pointer("/result/turn/id").and_then(Value::as_str),
    ) && json_id_key(id).is_some()
        && protocol_id_is_valid(turn_id)
    {
        return ServerProjection::Unique(json!({"id": id, "result": {"turn": {"id": turn_id}}}));
    }
    if let (Some(id), Some(thread_id)) = (value.get("id"), value.pointer("/result/thread/id")) {
        let Some(thread_id) = thread_id.as_str() else {
            return ServerProjection::RejectedUnique;
        };
        if json_id_key(id).is_none() || !protocol_id_is_valid(thread_id) {
            return ServerProjection::RejectedUnique;
        }
        let mut result = json!({"thread": {"id": thread_id}});
        // Codex 0.162.1 reports idle here even without a model or initial prompt.
        // Retain only the status needed for startup, never the thread contents.
        if value
            .pointer("/result/thread/status/type")
            .and_then(Value::as_str)
            == Some("idle")
        {
            result["thread"]["status"] = json!({"type":"idle"});
        }
        result.as_object_mut().expect("object").extend(
            project_model_settings(&value["result"])
                .as_object()
                .expect("object")
                .clone(),
        );
        return bounded_observation(json!({"id": id, "result": result}))
            .map(ServerProjection::Unique)
            .unwrap_or(ServerProjection::RejectedUnique);
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return ServerProjection::Irrelevant;
    };
    if let Some(projection) = exact_attention::nonblocking_input(value) {
        return projection;
    }
    if let Some(kind) = match method {
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval" => Some("approval"),
        "item/tool/requestUserInput" => Some("clarification"),
        "mcpServer/elicitation/request" => {
            match value.pointer("/params/mode").and_then(Value::as_str) {
                Some("form" | "openai/form" | "openaiForm") => Some("clarification"),
                Some("url") => Some("authentication"),
                _ => return ServerProjection::RejectedUnique,
            }
        }
        _ => None,
    } {
        let (Some(request_id), Some(thread_id)) = (
            value.get("id"),
            value.pointer("/params/threadId").and_then(Value::as_str),
        ) else {
            return ServerProjection::RejectedUnique;
        };
        let turn_id = value.pointer("/params/turnId");
        let turn_required = method != "mcpServer/elicitation/request";
        if attention_request_id_key(request_id).is_none()
            || !protocol_id_is_valid(thread_id)
            || (turn_required
                && !turn_id
                    .and_then(Value::as_str)
                    .is_some_and(protocol_id_is_valid))
            || (!turn_required
                && turn_id.is_some_and(|turn_id| {
                    !turn_id.is_null() && !turn_id.as_str().is_some_and(protocol_id_is_valid)
                }))
        {
            return ServerProjection::RejectedUnique;
        }
        return bounded_observation(json!({
            "method": "agent-session/attention/requested",
            "params": {
                "requestId": request_id,
                "threadId": thread_id,
                "turnId": turn_id,
                "kind": kind
            }
        }))
        .map(ServerProjection::Unique)
        .unwrap_or(ServerProjection::RejectedUnique);
    }
    if method == "serverRequest/resolved" {
        let (Some(request_id), Some(thread_id)) = (
            value.pointer("/params/requestId"),
            value.pointer("/params/threadId").and_then(Value::as_str),
        ) else {
            return ServerProjection::RejectedUnique;
        };
        if attention_request_id_key(request_id).is_none() || !protocol_id_is_valid(thread_id) {
            return ServerProjection::RejectedUnique;
        }
        return bounded_observation(json!({
            "method": "agent-session/attention/resolved",
            "params": {"requestId": request_id, "threadId": thread_id}
        }))
        .map(ServerProjection::Unique)
        .unwrap_or(ServerProjection::RejectedUnique);
    }
    let observation = match method {
        "error" => {
            let (Some(thread_id), Some(turn_id)) = (
                value.pointer("/params/threadId").and_then(Value::as_str),
                value.pointer("/params/turnId").and_then(Value::as_str),
            ) else {
                return ServerProjection::RejectedUnique;
            };
            if !protocol_id_is_valid(thread_id) || !protocol_id_is_valid(turn_id) {
                return ServerProjection::RejectedUnique;
            }
            json!({
                "method": "error",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "willRetry": value.pointer("/params/willRetry"),
                    "error": {
                        "codexErrorInfo": value.pointer("/params/error/codexErrorInfo")
                    }
                }
            })
        }
        "turn/completed" => {
            let (Some(thread_id), Some(turn_id)) = (
                value.pointer("/params/threadId").and_then(Value::as_str),
                value.pointer("/params/turn/id").and_then(Value::as_str),
            ) else {
                return ServerProjection::RejectedUnique;
            };
            if !protocol_id_is_valid(thread_id) || !protocol_id_is_valid(turn_id) {
                return ServerProjection::RejectedUnique;
            }
            json!({
                "method": "turn/completed",
                "params": {
                    "threadId": thread_id,
                    "turn": {
                        "id": turn_id,
                        "status": value.pointer("/params/turn/status"),
                        "error": {
                            "codexErrorInfo": value.pointer("/params/turn/error/codexErrorInfo")
                        }
                    }
                }
            })
        }
        "account/rateLimits/updated" => json!({
            "method": "account/rateLimits/updated",
            "params": {
                "rateLimits": value.pointer("/params/rateLimits"),
                "rateLimitsByLimitId": value.pointer("/params/rateLimitsByLimitId")
            }
        }),
        _ => return ServerProjection::Irrelevant,
    };
    match bounded_observation(observation) {
        Some(observation) if method == "account/rateLimits/updated" => {
            ServerProjection::Repeatable(observation)
        }
        Some(observation) => ServerProjection::Unique(observation),
        None if method == "account/rateLimits/updated" => ServerProjection::Irrelevant,
        None => ServerProjection::RejectedUnique,
    }
}
