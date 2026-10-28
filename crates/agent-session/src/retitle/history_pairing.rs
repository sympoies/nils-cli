use super::*;

pub(super) fn pair_turns(messages: Vec<HistoryMessage>) -> Vec<TitleContextTurn> {
    let mut turns: Vec<TitleContextTurn> = Vec::new();
    let mut assistant_allowed = false;
    for message in messages {
        match message.role.as_str() {
            // Image-only tool results add no text evidence or turn boundary.
            "user" if !message.human_prompt && message.text.trim().is_empty() => {}
            "assistant" if !message.images.is_empty() && message.text.trim().is_empty() => {}
            "user" if !message.human_prompt => assistant_allowed = false,
            "user" => {
                let prompt = filter_text(&message.text);
                if !prompt.is_empty() {
                    turns.push(TitleContextTurn {
                        id: message.id,
                        user_prompt: prompt,
                        assistant_excerpt: None,
                    });
                    assistant_allowed = true;
                }
            }
            "assistant" if assistant_allowed => {
                if let Some(turn) = turns.last_mut()
                    && turn.assistant_excerpt.is_none()
                {
                    let excerpt = filter_text(&message.text);
                    if !excerpt.is_empty() {
                        turn.assistant_excerpt = Some(excerpt);
                    }
                }
                assistant_allowed = false;
            }
            _ => {}
        }
    }
    turns
}

#[cfg(test)]
#[path = "history_pairing_tests.rs"]
mod tests;
