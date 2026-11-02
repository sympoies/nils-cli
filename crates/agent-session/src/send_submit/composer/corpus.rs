//! Frozen-classifier contract over recorded and derived panes. Each entry is a
//! capture with its escape bytes intact and the result the reader must keep
//! giving. An unknown form must stay a typed refusal; it is never made to pass
//! by widening the reader. Add a case only from a sanitized real capture or from
//! a named edit of one.
use super::{Unobserved, direct_composer_observation, exact_composer_input};
use serde::Deserialize;

#[derive(Deserialize)]
struct Corpus {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    provider: String,
    capture: String,
    exact: Option<String>,
    direct: Direct,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direct {
    Input(String),
    Observation(Unobserved),
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!("real-pane-corpus.json")).unwrap()
}

#[test]
fn recorded_panes_keep_their_classification() {
    let corpus = corpus();
    assert!(corpus.entries.len() >= 5);
    for entry in corpus.entries {
        let exact = exact_composer_input(&entry.provider, &entry.capture);
        assert_eq!(exact, entry.exact, "{} exact", entry.id);
        let direct = direct_composer_observation(&entry.provider, &entry.capture);
        let expected = match entry.direct {
            Direct::Input(input) => Ok(input),
            Direct::Observation(reason) => Err(reason),
        };
        assert_eq!(direct, expected, "{} direct", entry.id);
    }
}

#[test]
fn recorded_panes_carry_no_host_specific_text() {
    for entry in corpus().entries {
        for marker in ["/home/", "/Users/", "scratchpad", "probe-run", "@"] {
            assert!(
                !entry.capture.contains(marker),
                "{} holds {marker}",
                entry.id
            );
        }
    }
}
