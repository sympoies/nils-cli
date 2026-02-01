use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

pub fn execute_request(_request_json: &serde_json::Value) -> Result<HttpResponse> {
    anyhow::bail!("api-testing-core::http::execute_request is not implemented yet");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_request_reports_unimplemented() {
        let err = execute_request(&serde_json::json!({"method": "GET"})).unwrap_err();
        assert!(err
            .to_string()
            .contains("api-testing-core::http::execute_request is not implemented"));
    }
}
