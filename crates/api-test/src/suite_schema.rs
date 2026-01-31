use std::fmt;

use serde::Deserialize;

pub const SUITE_SCHEMA_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawText(pub String);

impl RawText {
    pub fn trimmed_lower(&self) -> String {
        self.0.trim().to_ascii_lowercase()
    }
}

impl<'de> Deserialize<'de> for RawText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = serde_json::Value::deserialize(deserializer)?;
        let s = match v {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        Ok(Self(s))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteManifestV1 {
    pub version: u32,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub defaults: Option<SuiteDefaultsV1>,
    #[serde(default)]
    pub auth: Option<SuiteAuthV1>,
    pub cases: Vec<SuiteCaseV1>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteDefaultsV1 {
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default, rename = "noHistory")]
    pub no_history: Option<RawText>,
    #[serde(default)]
    pub rest: Option<SuiteDefaultsRestV1>,
    #[serde(default)]
    pub graphql: Option<SuiteDefaultsGraphqlV1>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteDefaultsRestV1 {
    #[serde(default, rename = "configDir")]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteDefaultsGraphqlV1 {
    #[serde(default, rename = "configDir")]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub jwt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteAuthV1 {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub required: Option<RawText>,
    #[serde(default, rename = "secretEnv")]
    pub secret_env: Option<String>,
    #[serde(default)]
    pub rest: Option<SuiteAuthRestV1>,
    #[serde(default)]
    pub graphql: Option<SuiteAuthGraphqlV1>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteAuthRestV1 {
    #[serde(default, rename = "loginRequestTemplate")]
    pub login_request_template: Option<String>,
    #[serde(default, rename = "credentialsJq")]
    pub credentials_jq: Option<String>,
    #[serde(default, rename = "tokenJq")]
    pub token_jq: Option<String>,
    #[serde(default, rename = "configDir")]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteAuthGraphqlV1 {
    #[serde(default, rename = "loginOp")]
    pub login_op: Option<String>,
    #[serde(default, rename = "loginVarsTemplate")]
    pub login_vars_template: Option<String>,
    #[serde(default, rename = "credentialsJq")]
    pub credentials_jq: Option<String>,
    #[serde(default, rename = "tokenJq")]
    pub token_jq: Option<String>,
    #[serde(default, rename = "configDir")]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteCaseV1 {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub case_type: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default, rename = "noHistory")]
    pub no_history: Option<RawText>,
    #[serde(default, rename = "allowWrite")]
    pub allow_write: Option<RawText>,
    #[serde(default, rename = "configDir")]
    pub config_dir: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub jwt: Option<String>,

    // REST
    #[serde(default)]
    pub request: Option<String>,

    // REST flow
    #[serde(default, rename = "loginRequest")]
    pub login_request: Option<String>,
    #[serde(default, rename = "tokenJq")]
    pub token_jq: Option<String>,

    // GraphQL
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub vars: Option<String>,
    #[serde(default, rename = "expect")]
    pub graphql_expect: Option<SuiteGraphqlExpectV1>,
    #[serde(default, rename = "allowErrors")]
    pub allow_errors: Option<RawText>,

    // TODO(sprint>6): model cleanup steps once runner implementation lands.
    #[serde(default)]
    pub cleanup: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SuiteGraphqlExpectV1 {
    #[serde(default)]
    pub jq: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuiteSchemaValidationError {
    UnsupportedSuiteVersion { got: u32 },

    InvalidSuiteAuthSecretEnvEmpty,
    InvalidSuiteAuthSecretEnvNotEnvVarName { value: String },
    InvalidSuiteAuthRequiredNotBoolean,
    InvalidSuiteAuthProviderRequiredWhenBothPresent,
    InvalidSuiteAuthProviderValue { value: String },

    InvalidSuiteAuthRestMissingLoginRequestTemplate,
    InvalidSuiteAuthRestMissingCredentialsJq,

    InvalidSuiteAuthGraphqlMissingLoginOp,
    InvalidSuiteAuthGraphqlMissingLoginVarsTemplate,
    InvalidSuiteAuthGraphqlMissingCredentialsJq,

    CaseMissingId { index: usize },
    CaseMissingType { id: String },

    RestCaseMissingRequest { id: String },
    RestFlowCaseMissingLoginRequest { id: String },
    RestFlowCaseMissingRequest { id: String },

    GraphqlCaseMissingOp { id: String },
    GraphqlCaseAllowErrorsInvalid { id: String },
    GraphqlCaseAllowErrorsTrueRequiresExpectJq { id: String },

    UnknownCaseType { id: String, case_type: String },
}

impl fmt::Display for SuiteSchemaValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SuiteSchemaValidationError::UnsupportedSuiteVersion { got } => {
                write!(
                    f,
                    "Unsupported suite version: {got} (expected {SUITE_SCHEMA_VERSION_V1})"
                )
            }

            SuiteSchemaValidationError::InvalidSuiteAuthSecretEnvEmpty => write!(
                f,
                "Invalid suite auth block: .auth.secretEnv is empty"
            ),
            SuiteSchemaValidationError::InvalidSuiteAuthSecretEnvNotEnvVarName { value } => write!(
                f,
                "Invalid suite auth block: .auth.secretEnv must be a valid env var name (got: {value})"
            ),
            SuiteSchemaValidationError::InvalidSuiteAuthRequiredNotBoolean => {
                write!(f, "Invalid suite auth block: .auth.required must be boolean")
            }
            SuiteSchemaValidationError::InvalidSuiteAuthProviderRequiredWhenBothPresent => write!(
                f,
                "Invalid suite auth block: .auth.provider is required when both .auth.rest and .auth.graphql are present"
            ),
            SuiteSchemaValidationError::InvalidSuiteAuthProviderValue { value } => write!(
                f,
                "Invalid suite auth block: .auth.provider must be one of: rest, graphql (got: {value})"
            ),

            SuiteSchemaValidationError::InvalidSuiteAuthRestMissingLoginRequestTemplate => write!(
                f,
                "Invalid suite auth.rest block: missing loginRequestTemplate"
            ),
            SuiteSchemaValidationError::InvalidSuiteAuthRestMissingCredentialsJq => {
                write!(f, "Invalid suite auth.rest block: missing credentialsJq")
            }

            SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingLoginOp => {
                write!(f, "Invalid suite auth.graphql block: missing loginOp")
            }
            SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingLoginVarsTemplate => write!(
                f,
                "Invalid suite auth.graphql block: missing loginVarsTemplate"
            ),
            SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingCredentialsJq => write!(
                f,
                "Invalid suite auth.graphql block: missing credentialsJq"
            ),

            SuiteSchemaValidationError::CaseMissingId { index } => {
                write!(f, "Case is missing id at index {index}")
            }
            SuiteSchemaValidationError::CaseMissingType { id } => {
                write!(f, "Case '{id}' is missing type")
            }

            SuiteSchemaValidationError::RestCaseMissingRequest { id } => {
                write!(f, "REST case '{id}' is missing request")
            }
            SuiteSchemaValidationError::RestFlowCaseMissingLoginRequest { id } => {
                write!(f, "rest-flow case '{id}' is missing loginRequest")
            }
            SuiteSchemaValidationError::RestFlowCaseMissingRequest { id } => {
                write!(f, "rest-flow case '{id}' is missing request")
            }

            SuiteSchemaValidationError::GraphqlCaseMissingOp { id } => {
                write!(f, "GraphQL case '{id}' is missing op")
            }
            SuiteSchemaValidationError::GraphqlCaseAllowErrorsInvalid { id } => write!(
                f,
                "GraphQL case '{id}' has invalid allowErrors (expected boolean)"
            ),
            SuiteSchemaValidationError::GraphqlCaseAllowErrorsTrueRequiresExpectJq { id } => write!(
                f,
                "GraphQL case '{id}' with allowErrors=true must set expect.jq"
            ),

            SuiteSchemaValidationError::UnknownCaseType { id, case_type } => {
                write!(f, "Unknown case type '{case_type}' for case '{id}'")
            }
        }
    }
}

impl std::error::Error for SuiteSchemaValidationError {}

fn is_valid_env_var_name(name: &str) -> bool {
    let name = name.trim();
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn parse_bool_raw(raw: &RawText) -> Option<bool> {
    match raw.trimmed_lower().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn auth_provider_effective(
    auth: &SuiteAuthV1,
) -> Result<Option<String>, SuiteSchemaValidationError> {
    let provider_raw = auth
        .provider
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    if !provider_raw.is_empty() {
        return Ok(Some(provider_raw));
    }

    let has_rest = auth.rest.is_some();
    let has_graphql = auth.graphql.is_some();

    if has_rest && !has_graphql {
        return Ok(Some("rest".to_string()));
    }
    if !has_rest && has_graphql {
        return Ok(Some("graphql".to_string()));
    }

    Err(SuiteSchemaValidationError::InvalidSuiteAuthProviderRequiredWhenBothPresent)
}

impl SuiteManifestV1 {
    pub fn validate(&self) -> Result<(), SuiteSchemaValidationError> {
        if self.version != SUITE_SCHEMA_VERSION_V1 {
            return Err(SuiteSchemaValidationError::UnsupportedSuiteVersion { got: self.version });
        }

        if let Some(auth) = &self.auth {
            let secret_env = auth
                .secret_env
                .as_deref()
                .unwrap_or("API_TEST_AUTH_JSON")
                .trim()
                .to_string();
            if secret_env.is_empty() {
                return Err(SuiteSchemaValidationError::InvalidSuiteAuthSecretEnvEmpty);
            }
            if !is_valid_env_var_name(&secret_env) {
                return Err(
                    SuiteSchemaValidationError::InvalidSuiteAuthSecretEnvNotEnvVarName {
                        value: secret_env,
                    },
                );
            }

            if let Some(required) = &auth.required {
                if parse_bool_raw(required).is_none() {
                    return Err(SuiteSchemaValidationError::InvalidSuiteAuthRequiredNotBoolean);
                }
            }

            let mut provider = auth_provider_effective(auth)?;
            if let Some(p) = &provider {
                if p == "gql" {
                    provider = Some("graphql".to_string());
                }
            }

            match provider.as_deref() {
                None => {}
                Some("rest") => {
                    let rest = auth.rest.as_ref().ok_or(
                        SuiteSchemaValidationError::InvalidSuiteAuthRestMissingLoginRequestTemplate,
                    )?;

                    let login = rest
                        .login_request_template
                        .as_deref()
                        .unwrap_or_default()
                        .trim();
                    if login.is_empty() {
                        return Err(
                            SuiteSchemaValidationError::InvalidSuiteAuthRestMissingLoginRequestTemplate,
                        );
                    }
                    let creds = rest.credentials_jq.as_deref().unwrap_or_default().trim();
                    if creds.is_empty() {
                        return Err(
                            SuiteSchemaValidationError::InvalidSuiteAuthRestMissingCredentialsJq,
                        );
                    }
                }
                Some("graphql") => {
                    let gql = auth
                        .graphql
                        .as_ref()
                        .ok_or(SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingLoginOp)?;

                    let login_op = gql.login_op.as_deref().unwrap_or_default().trim();
                    if login_op.is_empty() {
                        return Err(
                            SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingLoginOp,
                        );
                    }
                    let login_vars = gql
                        .login_vars_template
                        .as_deref()
                        .unwrap_or_default()
                        .trim();
                    if login_vars.is_empty() {
                        return Err(
                            SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingLoginVarsTemplate,
                        );
                    }
                    let creds = gql.credentials_jq.as_deref().unwrap_or_default().trim();
                    if creds.is_empty() {
                        return Err(
                            SuiteSchemaValidationError::InvalidSuiteAuthGraphqlMissingCredentialsJq,
                        );
                    }
                }
                Some(other) => {
                    return Err(SuiteSchemaValidationError::InvalidSuiteAuthProviderValue {
                        value: other.to_string(),
                    });
                }
            }
        }

        for (index, case) in self.cases.iter().enumerate() {
            let id = case.id.as_deref().unwrap_or_default().trim().to_string();
            if id.is_empty() {
                return Err(SuiteSchemaValidationError::CaseMissingId { index });
            }

            let case_type_raw = case
                .case_type
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_string();
            let case_type = case_type_raw.to_ascii_lowercase();
            if case_type.is_empty() {
                return Err(SuiteSchemaValidationError::CaseMissingType { id });
            }

            match case_type.as_str() {
                "rest" => {
                    let request = case.request.as_deref().unwrap_or_default().trim();
                    if request.is_empty() {
                        return Err(SuiteSchemaValidationError::RestCaseMissingRequest { id });
                    }
                }
                "rest-flow" | "rest_flow" => {
                    let login = case.login_request.as_deref().unwrap_or_default().trim();
                    if login.is_empty() {
                        return Err(
                            SuiteSchemaValidationError::RestFlowCaseMissingLoginRequest { id },
                        );
                    }
                    let request = case.request.as_deref().unwrap_or_default().trim();
                    if request.is_empty() {
                        return Err(SuiteSchemaValidationError::RestFlowCaseMissingRequest { id });
                    }
                }
                "graphql" => {
                    let op = case.op.as_deref().unwrap_or_default().trim();
                    if op.is_empty() {
                        return Err(SuiteSchemaValidationError::GraphqlCaseMissingOp { id });
                    }

                    let allow_errors = case.allow_errors.as_ref();
                    let allow_errors_value = match allow_errors {
                        None => false,
                        Some(raw) => match parse_bool_raw(raw) {
                            Some(v) => v,
                            None => {
                                return Err(
                                    SuiteSchemaValidationError::GraphqlCaseAllowErrorsInvalid {
                                        id,
                                    },
                                );
                            }
                        },
                    };

                    if allow_errors_value {
                        let expect_jq = case
                            .graphql_expect
                            .as_ref()
                            .and_then(|e| e.jq.as_deref())
                            .unwrap_or_default()
                            .trim();
                        if expect_jq.is_empty() {
                            return Err(
                                SuiteSchemaValidationError::GraphqlCaseAllowErrorsTrueRequiresExpectJq { id },
                            );
                        }
                    }
                }
                _ => {
                    return Err(SuiteSchemaValidationError::UnknownCaseType {
                        id,
                        case_type: case_type_raw,
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_schema_v1_accepts_minimal_valid_suite() {
        let suite: SuiteManifestV1 = serde_json::from_value(serde_json::json!({
            "version": 1,
            "name": "smoke",
            "cases": [
                { "id": "rest.health", "type": "rest", "request": "setup/rest/requests/health.request.json" },
                { "id": "graphql.health", "type": "graphql", "op": "setup/graphql/ops/health.graphql" }
            ]
        }))
        .unwrap();
        suite.validate().unwrap();
    }

    #[test]
    fn suite_schema_v1_graphql_allow_errors_true_requires_expect_jq() {
        let suite: SuiteManifestV1 = serde_json::from_value(serde_json::json!({
            "version": 1,
            "cases": [
                { "id": "graphql.bad", "type": "graphql", "op": "x.graphql", "allowErrors": true }
            ]
        }))
        .unwrap();
        let err = suite.validate().unwrap_err();
        assert_eq!(
            err,
            SuiteSchemaValidationError::GraphqlCaseAllowErrorsTrueRequiresExpectJq {
                id: "graphql.bad".to_string()
            }
        );
        assert!(err.to_string().contains("graphql.bad"));
    }

    #[test]
    fn suite_schema_v1_graphql_allow_errors_must_be_boolean() {
        let suite: SuiteManifestV1 = serde_json::from_value(serde_json::json!({
            "version": 1,
            "cases": [
                { "id": "graphql.bad", "type": "graphql", "op": "x.graphql", "allowErrors": "maybe" }
            ]
        }))
        .unwrap();
        let err = suite.validate().unwrap_err();
        assert_eq!(
            err,
            SuiteSchemaValidationError::GraphqlCaseAllowErrorsInvalid {
                id: "graphql.bad".to_string()
            }
        );
        assert!(err.to_string().contains("graphql.bad"));
    }

    #[test]
    fn suite_schema_v1_unknown_case_type_includes_case_id() {
        let suite: SuiteManifestV1 = serde_json::from_value(serde_json::json!({
            "version": 1,
            "cases": [
                { "id": "x", "type": "soap" }
            ]
        }))
        .unwrap();
        let err = suite.validate().unwrap_err();
        assert!(err.to_string().contains("case 'x'"));
        assert!(err.to_string().contains("soap"));
    }

    #[test]
    fn suite_schema_v1_rest_flow_requires_login_request_and_request() {
        let suite: SuiteManifestV1 = serde_json::from_value(serde_json::json!({
            "version": 1,
            "cases": [
                { "id": "rest.flow", "type": "rest-flow", "request": "x.request.json" }
            ]
        }))
        .unwrap();
        let err = suite.validate().unwrap_err();
        assert_eq!(
            err,
            SuiteSchemaValidationError::RestFlowCaseMissingLoginRequest {
                id: "rest.flow".to_string()
            }
        );
    }

    #[test]
    fn suite_schema_v1_auth_secret_env_must_be_valid_env_var_name() {
        let suite: SuiteManifestV1 = serde_json::from_value(serde_json::json!({
            "version": 1,
            "auth": { "secretEnv": "123" },
            "cases": [
                { "id": "rest.health", "type": "rest", "request": "x.request.json" }
            ]
        }))
        .unwrap();
        let err = suite.validate().unwrap_err();
        assert!(err.to_string().contains(".auth.secretEnv"));
    }
}
