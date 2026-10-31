use nils_common::cli_contract::{Envelope, EnvelopeError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const REQUEST_SCHEMA: &str = "research.request.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Openai,
    Anthropic,
}
impl Provider {
    pub fn model(self) -> &'static str {
        match self {
            Self::Openai => "gpt-6-luna",
            Self::Anthropic => "claude-haiku-5-5",
        }
    }
    pub fn key_name(self) -> &'static str {
        match self {
            Self::Openai => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
        }
    }
    pub fn price_source(self) -> &'static str {
        match self {
            Self::Openai => "https://developers.openai.com/api/docs/pricing",
            Self::Anthropic => "https://platform.claude.com/docs/en/about-claude/pricing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    Search,
    ReadUrl,
    Brief,
}
impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::ReadUrl => "read-url",
            Self::Brief => "brief",
        }
    }
    pub fn schema(self) -> String {
        format!("cli.research-cli.{}.v1", self.name())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub timeout_seconds: u64,
    pub max_bytes: usize,
    pub max_chars: usize,
    pub max_redirects: usize,
    pub max_output_tokens: u32,
    pub max_tool_uses: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            max_bytes: 262_144,
            max_chars: 16_384,
            max_redirects: 5,
            max_output_tokens: 1024,
            max_tool_uses: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: String,
    pub operation: Operation,
    #[serde(default)]
    pub query: String,
    pub provider: Provider,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub recency_days: Option<u32>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub limits: Limits,
}
fn default_limit() -> usize {
    5
}
impl Request {
    pub fn new(operation: Operation, query: impl Into<String>, provider: Provider) -> Self {
        Self {
            schema_version: REQUEST_SCHEMA.into(),
            operation,
            query: query.into(),
            provider,
            language: None,
            domains: vec![],
            recency_days: None,
            limit: default_limit(),
            urls: vec![],
            fields: vec![],
            limits: Limits::default(),
        }
    }
    pub fn validate(&self) -> Result<(), ResearchError> {
        let l = &self.limits;
        if self.schema_version != REQUEST_SCHEMA
            || self.query.len() > 16_384
            || self.limit == 0
            || self.limit > 50
            || self.urls.len() > 10
            || self.domains.len() > 100
            || self.fields.len() > 30
            || l.timeout_seconds == 0
            || l.timeout_seconds > 300
            || l.max_bytes == 0
            || l.max_bytes > 4_194_304
            || l.max_chars == 0
            || l.max_chars > 262_144
            || l.max_redirects > 10
            || l.max_output_tokens == 0
            || l.max_output_tokens > 8192
            || l.max_tool_uses == 0
            || l.max_tool_uses > 10
            || self.recency_days == Some(0)
            || self
                .language
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 80)
            || self.fields.iter().any(|s| s.is_empty() || s.len() > 200)
            || self.urls.iter().any(|s| s.len() > 8192)
            || self.domains.iter().any(|s| !valid_domain(s))
        {
            return Err(ResearchError::new(
                "invalid-request",
                "Invalid schema, filter or limit",
            ));
        }
        match self.operation {
            Operation::ReadUrl if self.urls.len() != 1 || !self.query.is_empty() => {
                Err(ResearchError::new(
                    "invalid-request",
                    "read-url requires exactly one URL and no query",
                ))
            }
            Operation::Search | Operation::Brief if self.query.trim().is_empty() => Err(
                ResearchError::new("invalid-request", "A nonempty query is required"),
            ),
            Operation::Search if !self.urls.is_empty() => Err(ResearchError::new(
                "invalid-request",
                "Use brief to supply URLs",
            )),
            _ => Ok(()),
        }
    }
}
fn valid_domain(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub url: String,
    pub final_url: Option<String>,
    pub title: Option<String>,
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub claim: String,
    pub source_ids: Vec<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assessment {
    pub label: String,
    pub generated: bool,
    pub key_points: Vec<String>,
    pub suggested_directions: Vec<String>,
    pub preliminary_judgment: Option<String>,
    pub confidence: Confidence,
}
impl Default for Assessment {
    fn default() -> Self {
        Self {
            label: "model_inference".into(),
            generated: false,
            key_points: vec![],
            suggested_directions: vec![],
            preliminary_judgment: None,
            confidence: Confidence::Low,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub search_calls: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd_estimate: Option<f64>,
    pub price_source: Option<String>,
    pub raw: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    pub query: String,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub retrieved_at: String,
    pub sources: Vec<Source>,
    pub facts: Vec<Fact>,
    pub summary: Option<String>,
    pub assessment: Assessment,
    pub unknowns: Vec<String>,
    pub conflicts: Vec<String>,
    pub usage: Usage,
    pub warnings: Vec<String>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}
impl ResearchResult {
    pub fn empty(request: &Request) -> Self {
        let model = request.operation != Operation::ReadUrl;
        Self {
            query: request.query.clone(),
            provider: model.then_some(request.provider),
            model: model.then(|| request.provider.model().into()),
            retrieved_at: jiff::Timestamp::now().to_string(),
            sources: vec![],
            facts: vec![],
            summary: None,
            assessment: Assessment::default(),
            unknowns: vec![],
            conflicts: vec![],
            usage: Usage {
                price_source: model.then(|| request.provider.price_source().into()),
                ..Usage::default()
            },
            warnings: vec![],
            truncated: false,
            content: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResearchError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}
impl ResearchError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }
    pub fn status(status: u16, request_id: Option<String>) -> Self {
        let code = match status {
            429 => "rate-limited",
            400 | 404 | 422 => "unsupported-capability",
            401 | 403 => "provider-auth",
            _ => "provider-error",
        };
        Self {
            code,
            message: "Provider request failed; no retry or substitution was attempted".into(),
            details: Some(json!({"http_status":status,"request_id":request_id})),
        }
    }
    pub fn envelope(
        self,
        operation: Operation,
        partial: Option<ResearchResult>,
    ) -> Envelope<ResearchResult> {
        let mut error = EnvelopeError::new(self.code, self.message);
        error.details = self.details;
        let mut envelope = Envelope::failure(operation.schema(), error);
        if let Some(data) = partial {
            envelope.warnings.clone_from(&data.warnings);
            envelope.data = Some(data);
        }
        envelope
    }
}
