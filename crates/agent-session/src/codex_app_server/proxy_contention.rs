use super::*;

pub(super) fn tui_busy_response(id: &Value, rejection: TuiMutationRejection) -> Message {
    Message::Text(
        json!({
            "id": id,
            "error": {
                "code": -32001,
                "message": "agent-session state is busy; retry the request",
                "data": {
                    "reason": rejection.code(),
                    "retryable": matches!(rejection, TuiMutationRejection::ManualCancellationBusy | TuiMutationRejection::TurnGateBusy),
                    "next_action": if matches!(rejection, TuiMutationRejection::ManualCancellationBusy | TuiMutationRejection::TurnGateBusy) {
                        "retry-request-after-state-change"
                    } else {
                        "inspect-session-state"
                    },
                    "recovery": { "outcome": "not_forwarded" }
                }
            }
        })
        .to_string()
        .into(),
    )
}
