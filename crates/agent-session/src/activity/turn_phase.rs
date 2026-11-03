use serde::{Deserialize, Serialize};

/// Current `agent-session.turn-state.v1` phase vocabulary.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    Starting,
    Working,
    Waiting,
    NeedsInput,
    Unresponsive,
    Unknown,
}
