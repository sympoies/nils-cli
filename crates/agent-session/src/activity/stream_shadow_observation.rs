use serde::Serialize;

#[derive(Clone, Debug, Serialize, Eq)]
pub(crate) struct StreamShadowObservationView {
    pub(crate) observer_version: String,
    pub(crate) rule_id: String,
    pub(crate) observed_at: String,
    pub(crate) projection: String,
    pub(crate) disagrees: bool,
}

// A fresh sampling timestamp alone is not a stream state change. Keep it in
// serialized snapshots without broadcasting the same observation repeatedly.
impl PartialEq for StreamShadowObservationView {
    fn eq(&self, other: &Self) -> bool {
        self.observer_version == other.observer_version
            && self.rule_id == other.rule_id
            && self.projection == other.projection
            && self.disagrees == other.disagrees
    }
}
