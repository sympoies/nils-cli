use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NotSelected,
    SkippedById,
    TagMismatch,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotSelected => "not_selected",
            Self::SkippedById => "skipped_by_id",
            Self::TagMismatch => "tag_mismatch",
        }
    }
}

pub fn parse_csv_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn case_matches_tags(case_tags: &[String], required_tags: &[String]) -> bool {
    if required_tags.is_empty() {
        return true;
    }
    let tags: HashSet<&str> = case_tags.iter().map(|s| s.as_str()).collect();
    required_tags.iter().all(|t| tags.contains(t.as_str()))
}

pub fn selection_skip_reason(
    case_id: &str,
    case_tags: &[String],
    required_tags: &[String],
    only_ids: &HashSet<String>,
    skip_ids: &HashSet<String>,
) -> Option<SkipReason> {
    let mut reason: Option<SkipReason> = None;

    if !only_ids.is_empty() && !only_ids.contains(case_id) {
        reason = Some(SkipReason::NotSelected);
    }
    if skip_ids.contains(case_id) {
        reason = Some(SkipReason::SkippedById);
    }
    if reason.is_none() && !case_matches_tags(case_tags, required_tags) {
        reason = Some(SkipReason::TagMismatch);
    }

    reason
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn suite_filter_tags_are_and_semantics() {
        let tags = vec!["smoke".to_string(), "shard:0".to_string()];
        assert!(case_matches_tags(&tags, &["smoke".to_string()]));
        assert!(case_matches_tags(
            &tags,
            &["smoke".to_string(), "shard:0".to_string()]
        ));
        assert!(!case_matches_tags(
            &tags,
            &["smoke".to_string(), "shard:1".to_string()]
        ));
    }

    #[test]
    fn suite_filter_only_and_skip_reason_precedence_matches_script() {
        let mut only_ids = HashSet::new();
        only_ids.insert("a".to_string());
        let mut skip_ids = HashSet::new();
        skip_ids.insert("b".to_string());

        let tags: Vec<String> = Vec::new();

        assert_eq!(
            selection_skip_reason("c", &tags, &[], &only_ids, &skip_ids),
            Some(SkipReason::NotSelected)
        );

        assert_eq!(
            selection_skip_reason("b", &tags, &[], &only_ids, &skip_ids),
            Some(SkipReason::SkippedById)
        );
    }
}
