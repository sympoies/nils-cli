use super::*;
use pretty_assertions::assert_eq;

pub(crate) fn seed(context: &CliContext, record: &SessionRecord) {
    let mut locked = lock_registry(context).unwrap();
    let incarnation = incarnation(record).unwrap();
    locked.registry.claims.push(serde_json::from_value(serde_json::json!({
        "schema_version":"agent-session.work-context.v1","session_id":record.id,"session_incarnation":incarnation,"claim_id":"fixture-claim","revision":1,"state":"active","intent":"execute","tier":"task","repositories":[],"worktrees":[],"provider_refs":[],"plan_refs":[],"scopes":[],"summary":"","updated_at":"2030-01-01T00:00:00Z","expires_at":"2030-01-01T00:30:00Z","expires_at_epoch":jiff::Timestamp::now().as_second()+1800
    })).unwrap());
    locked.registry.operations.push(serde_json::from_value(serde_json::json!({
        "schema_version":"agent-session.operation-lease.v1","lease_id":"fixture-lease","session_id":record.id,"session_incarnation":incarnation,"claim_id":"fixture-claim","claim_revision":1,"operation":"edit","targets":[],"state":"active","revision":1,"started_at":"2030-01-01T00:00:00Z","expires_at":"2030-01-01T00:30:00Z","expires_at_epoch":jiff::Timestamp::now().as_second()+1800,"execution_token_digest":"sensitive-canary"
    })).unwrap());
    let mut abandoned = locked.registry.operations.last().unwrap().clone();
    abandoned.lease_id = "already-retired-lease".into();
    abandoned.state = "abandoned".into();
    abandoned.revision = 2;
    locked.registry.operations.push(abandoned);
    locked.save().unwrap();
}

pub(crate) fn assert_retired(context: &CliContext, record: &SessionRecord) {
    let locked = lock_registry_observational(context).unwrap();
    let incarnation = incarnation(record).unwrap();
    assert!(
        locked
            .registry
            .claims
            .iter()
            .filter(|c| c.session_incarnation == incarnation)
            .all(|c| c.state == "released")
    );
    let operation = locked
        .registry
        .operations
        .iter()
        .find(|op| op.session_incarnation == incarnation)
        .unwrap();
    assert_eq!(operation.state, "abandoned");
    assert_eq!(operation.outcome, None);
}
