use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlAssertions {
    pub default_no_errors: String,
    pub default_has_data: Option<String>,
    pub jq: Option<String>,
}

impl GraphqlAssertions {
    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "defaultNoErrors".to_string(),
            serde_json::Value::String(self.default_no_errors.clone()),
        );
        if let Some(v) = &self.default_has_data {
            obj.insert(
                "defaultHasData".to_string(),
                serde_json::Value::String(v.clone()),
            );
        }
        if let Some(v) = &self.jq {
            obj.insert("jq".to_string(), serde_json::Value::String(v.clone()));
        }
        serde_json::Value::Object(obj)
    }
}

fn errors_len(value: &serde_json::Value) -> usize {
    value
        .get("errors")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

pub fn evaluate_graphql_response_for_suite(
    response_json: &serde_json::Value,
    allow_errors: bool,
    expect_jq: Option<&str>,
) -> Result<GraphqlAssertions> {
    let default_no_errors = if errors_len(response_json) == 0 {
        "passed"
    } else {
        "failed"
    };

    let default_has_data = if !allow_errors && expect_jq.is_none() {
        let ok = response_json
            .get("data")
            .is_some_and(|v| !v.is_null() && v.is_object());
        Some(if ok { "passed" } else { "failed" }.to_string())
    } else {
        None
    };

    let jq = if let Some(expr) = expect_jq {
        let ok = crate::jq::eval_exit_status(response_json, expr).unwrap_or(false);
        Some(if ok { "passed" } else { "failed" }.to_string())
    } else {
        None
    };

    Ok(GraphqlAssertions {
        default_no_errors: default_no_errors.to_string(),
        default_has_data,
        jq,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_expect_default_no_errors_and_has_data() {
        let v = serde_json::json!({"data": {"x": 1}});
        let a = evaluate_graphql_response_for_suite(&v, false, None).unwrap();
        assert_eq!(a.default_no_errors, "passed");
        assert_eq!(a.default_has_data.as_deref(), Some("passed"));
        assert!(a.jq.is_none());
    }

    #[test]
    fn graphql_expect_errors_present_marks_failed() {
        let v = serde_json::json!({"data": {"x": 1}, "errors": [{"message": "no"}]});
        let a = evaluate_graphql_response_for_suite(&v, false, None).unwrap();
        assert_eq!(a.default_no_errors, "failed");
    }

    #[test]
    fn graphql_expect_jq_is_evaluated() {
        let v = serde_json::json!({"data": {"ok": true}});
        let a = evaluate_graphql_response_for_suite(&v, false, Some(".data.ok == true")).unwrap();
        assert_eq!(a.jq.as_deref(), Some("passed"));
        assert!(a.default_has_data.is_none());
    }
}
