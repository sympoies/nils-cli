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
