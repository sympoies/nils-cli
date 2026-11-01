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
    let exact = content.len() == 1
        && content[0]["type"] == "text"
        && content[0]["text"].as_str().is_some_and(matches_text);
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
