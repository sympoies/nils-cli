//! One exact acknowledged turn and one exact user input are required everywhere.
use super::*;

pub(super) fn input_observed(
    history: &Value,
    thread_id: &str,
    turn_id: &str,
    text: &str,
) -> trace::Observation {
    input_observed_by(history, thread_id, turn_id, |actual| actual == text)
}

pub(super) fn input_observed_by(
    history: &Value,
    thread_id: &str,
    turn_id: &str,
    matches_text: impl FnOnce(&str) -> bool,
) -> trace::Observation {
    input_observed_by_content(history, thread_id, turn_id, |content| {
        content.len() == 1
            && content[0]["type"] == "text"
            && content[0]["text"].as_str().is_some_and(matches_text)
    })
}

pub(super) fn image_input_observed(
    history: &Value,
    thread_id: &str,
    turn_id: &str,
    expected: &[Value],
) -> trace::Observation {
    // Codex 0.171.0 thread history can coalesce text and append the ordered
    // images. Compare the exact text and exact URL sequence in that projection.
    let expected = image_content(expected);
    input_observed_by_content(history, thread_id, turn_id, |content| {
        expected.is_some() && image_content(content) == expected
    })
}

fn image_content(content: &[Value]) -> Option<(String, Vec<&str>)> {
    let mut text = String::new();
    let mut images = Vec::new();
    for part in content {
        match part["type"].as_str()? {
            "text" => text.push_str(part["text"].as_str()?),
            "image" => images.push(part["url"].as_str()?),
            _ => return None,
        }
    }
    if text.trim().is_empty() {
        text.clear();
    }
    Some((text, images))
}

fn input_observed_by_content(
    history: &Value,
    thread_id: &str,
    turn_id: &str,
    matches_content: impl FnOnce(&[Value]) -> bool,
) -> trace::Observation {
    let mut observed = trace::Observation::invalid();
    observed.thread_matches =
        Some(history.pointer("/thread/id").and_then(Value::as_str) == Some(thread_id));
    if observed.thread_matches != Some(true) {
        return observed;
    }
    let Some(turns) = history.pointer("/thread/turns").and_then(Value::as_array) else {
        return observed;
    };
    let mut matching = turns
        .iter()
        .filter(|turn| turn["id"].as_str() == Some(turn_id));
    let Some(turn) = matching.next() else {
        observed.turn_matches = Some(false);
        observed.result = Ok(false);
        return observed;
    };
    observed.turn_matches = Some(matching.next().is_none());
    if observed.turn_matches != Some(true) {
        return observed;
    }
    let Some(items) = turn["items"].as_array() else {
        return observed;
    };
    let mut inputs = items.iter().filter(|item| item["type"] == "userMessage");
    let Some(input) = inputs.next() else {
        observed.result = Ok(false);
        return observed;
    };
    if inputs.next().is_some() {
        return observed;
    }
    let Some(content) = input["content"].as_array() else {
        return observed;
    };
    let exact = matches_content(content);
    observed.text_matches = Some(exact);
    if exact {
        observed.result = Ok(true);
    }
    observed
}

/// The turn holding the one user message that carries `client_id`, found
/// without the acknowledged turn id. The id alone is not enough: the message
/// must be exactly one text part that the caller accepts. A duplicate id, a
/// different text or any unexpected shape is no match.
pub(super) fn input_by_client_id(
    history: &Value,
    thread_id: &str,
    client_id: &str,
    matches_text: impl Fn(&str) -> bool,
) -> Option<String> {
    if history.pointer("/thread/id").and_then(Value::as_str) != Some(thread_id) {
        return None;
    }
    let mut found = None;
    for turn in history.pointer("/thread/turns")?.as_array()? {
        for item in turn["items"].as_array()? {
            if item["type"] != "userMessage" || item["clientId"].as_str() != Some(client_id) {
                continue;
            }
            let content = item["content"].as_array()?;
            let exact = content.len() == 1
                && content[0]["type"] == "text"
                && content[0]["text"].as_str().is_some_and(&matches_text);
            if !exact || found.is_some() {
                return None;
            }
            found = turn["id"].as_str().map(str::to_owned);
        }
    }
    found.filter(|id| protocol_id_is_valid(id))
}
