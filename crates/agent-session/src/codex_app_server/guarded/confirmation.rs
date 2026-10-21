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
