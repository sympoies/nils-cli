use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;

use crate::suite::resolve::{
    resolve_gql_url_for_env, resolve_path_from_repo_root, resolve_rest_base_url_for_env,
};
use crate::suite::schema::{SuiteAuth, SuiteDefaults};
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProvider {
    Rest,
    Graphql,
}

#[derive(Debug)]
pub enum AuthInit {
    Disabled { message: Option<String> },
    Enabled(Box<SuiteAuthManager>),
}

#[derive(Debug)]
pub struct SuiteAuthManager {
    provider: AuthProvider,
    provider_label: String,
    secret_json: serde_json::Value,
    auth: SuiteAuth,
    tokens: HashMap<String, String>,
    errors: HashMap<String, String>,
}

impl SuiteAuthManager {
    pub fn provider_label(&self) -> &str {
        &self.provider_label
    }

    pub fn init_from_suite(auth: SuiteAuth, suite_defaults: &SuiteDefaults) -> Result<AuthInit> {
        let provider = canonical_provider(&auth)?;
        let provider_label = match provider {
            AuthProvider::Rest => "rest".to_string(),
            AuthProvider::Graphql => "graphql".to_string(),
        };

        let secret_env = auth.secret_env.trim().to_string();
        if secret_env.is_empty() {
            anyhow::bail!("Invalid suite auth block: .auth.secretEnv is empty");
        }

        let raw = std::env::var(&secret_env).ok().unwrap_or_default();
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            if !auth.required {
                return Ok(AuthInit::Disabled {
                    message: Some(format!(
                        "api-test-runner: auth disabled (missing {secret_env} and auth.required=false)"
                    )),
                });
            }
            anyhow::bail!("Missing auth secret env var for suite auth: {secret_env}");
        }

        let secret_json: serde_json::Value =
            serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in {secret_env}"))?;

        // Inherit auth configDir from suite defaults when omitted (parity with api-test.sh).
        let auth = inherit_auth_defaults(auth, suite_defaults);

        Ok(AuthInit::Enabled(Box::new(SuiteAuthManager {
            provider,
            provider_label,
            secret_json,
            auth,
            tokens: HashMap::new(),
            errors: HashMap::new(),
        })))
    }

    pub fn ensure_token(
        &mut self,
        profile: &str,
        repo_root: &Path,
        suite_defaults: &SuiteDefaults,
        env_rest_url: &str,
        env_gql_url: &str,
    ) -> std::result::Result<String, String> {
        self.ensure_token_with_login(profile, |mgr, profile| match mgr.provider {
            AuthProvider::Rest => mgr.login_rest(profile, repo_root, suite_defaults, env_rest_url),
            AuthProvider::Graphql => {
                mgr.login_graphql(profile, repo_root, suite_defaults, env_gql_url)
            }
        })
    }

    fn ensure_token_with_login<F>(
        &mut self,
        profile: &str,
        login: F,
    ) -> std::result::Result<String, String>
    where
        F: FnOnce(&SuiteAuthManager, &str) -> std::result::Result<String, String>,
    {
        let profile = profile.trim();
        if profile.is_empty() {
            return Err(format!(
                "auth_login_failed(provider={},profile=)",
                self.provider_label
            ));
        }

        if let Some(token) = self.tokens.get(profile) {
            return Ok(token.clone());
        }
        if let Some(err) = self.errors.get(profile) {
            return Err(err.clone());
        }

        let result = {
            let mgr: &SuiteAuthManager = &*self;
            login(mgr, profile)
        };

        match result {
            Ok(token) => {
                self.tokens.insert(profile.to_string(), token.clone());
                Ok(token)
            }
            Err(err) => {
                let fallback = format!(
                    "auth_login_failed(provider={},profile={profile})",
                    self.provider_label
                );
                let err = if err.trim().is_empty() { fallback } else { err };
                self.errors.insert(profile.to_string(), err.clone());
                Err(err)
            }
        }
    }

    fn render_credentials(
        &self,
        profile: &str,
        expr: &str,
        provider: &str,
    ) -> std::result::Result<serde_json::Value, String> {
        let mut vars = std::collections::BTreeMap::new();
        vars.insert(
            "profile".to_string(),
            serde_json::Value::String(profile.to_string()),
        );

        let out = crate::jq::query_with_vars(&self.secret_json, expr, &vars).map_err(|_| {
            format!("auth_credentials_jq_error(provider={provider},profile={profile})")
        })?;

        if out.is_empty() {
            return Err(format!(
                "auth_credentials_missing(provider={provider},profile={profile})"
            ));
        }
        if out.len() != 1 {
            return Err(format!(
                "auth_credentials_ambiguous(provider={provider},profile={profile},count={})",
                out.len()
            ));
        }

        let v = out.into_iter().next().unwrap_or(serde_json::Value::Null);
        match v {
            serde_json::Value::Object(_) => Ok(v),
            serde_json::Value::Null => Err(format!(
                "auth_credentials_missing(provider={provider},profile={profile})"
            )),
            _ => Err(format!(
                "auth_credentials_invalid(provider={provider},profile={profile})"
            )),
        }
    }

    fn extract_token(
        &self,
        response_json: &serde_json::Value,
        token_expr: &str,
        provider: &str,
        profile: &str,
    ) -> std::result::Result<String, String> {
        let token = crate::jq::query_raw(response_json, token_expr)
            .map_err(|_| format!("auth_token_jq_error(provider={provider},profile={profile})"))?
            .into_iter()
            .next()
            .unwrap_or_default();

        let token = token.trim().to_string();
        if token.is_empty() || token == "null" {
            return Err(format!(
                "auth_token_missing(provider={provider},profile={profile})"
            ));
        }
        Ok(token)
    }

    fn login_rest(
        &self,
        profile: &str,
        repo_root: &Path,
        suite_defaults: &SuiteDefaults,
        env_rest_url: &str,
    ) -> std::result::Result<String, String> {
        let Some(rest) = &self.auth.rest else {
            return Err(String::new());
        };

        let provider = "rest";
        let creds = self.render_credentials(profile, &rest.credentials_jq, provider)?;

        let template_path = resolve_path_from_repo_root(repo_root, &rest.login_request_template);
        if !template_path.is_file() {
            return Err(format!(
                "auth_login_template_render_failed(provider={provider},profile={profile})"
            ));
        }

        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&template_path).map_err(|_| {
                format!("auth_login_template_render_failed(provider={provider},profile={profile})")
            })?)
            .map_err(|_| {
                format!("auth_login_template_render_failed(provider={provider},profile={profile})")
            })?;

        let mut obj = raw.as_object().cloned().ok_or_else(|| {
            format!("auth_login_template_render_failed(provider={provider},profile={profile})")
        })?;

        let body = obj
            .get("body")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        let mut body_obj = match body {
            serde_json::Value::Null => serde_json::Map::new(),
            serde_json::Value::Object(m) => m,
            _ => {
                return Err(format!(
                    "auth_login_template_render_failed(provider={provider},profile={profile})"
                ))
            }
        };

        let serde_json::Value::Object(creds_obj) = creds else {
            return Err(format!(
                "auth_login_template_render_failed(provider={provider},profile={profile})"
            ));
        };
        for (k, v) in creds_obj {
            body_obj.insert(k, v);
        }
        obj.insert("body".to_string(), serde_json::Value::Object(body_obj));

        let request = crate::rest::schema::parse_rest_request_json(serde_json::Value::Object(obj))
            .map_err(|_| {
                format!("auth_login_template_render_failed(provider={provider},profile={profile})")
            })?;

        let request_file = crate::rest::schema::RestRequestFile {
            path: template_path.clone(),
            request,
        };

        let base_url = resolve_auth_rest_base_url(
            repo_root,
            &rest.config_dir,
            &rest.url,
            &rest.env,
            suite_defaults,
            env_rest_url,
        )
        .map_err(|_| {
            format!("auth_login_request_failed(provider={provider},profile={profile},rc=1)")
        })?;

        let executed = crate::rest::runner::execute_rest_request(&request_file, &base_url, None)
            .map_err(|_| {
                format!("auth_login_request_failed(provider={provider},profile={profile},rc=1)")
            })?;
        crate::rest::expect::evaluate_main_response(&request_file.request, &executed).map_err(
            |_| format!("auth_login_request_failed(provider={provider},profile={profile},rc=1)"),
        )?;

        let response_json: serde_json::Value = serde_json::from_slice(&executed.response.body)
            .map_err(|_| format!("auth_token_jq_error(provider={provider},profile={profile})"))?;

        self.extract_token(&response_json, &rest.token_jq, provider, profile)
    }

    fn login_graphql(
        &self,
        profile: &str,
        repo_root: &Path,
        suite_defaults: &SuiteDefaults,
        env_gql_url: &str,
    ) -> std::result::Result<String, String> {
        let Some(gql) = &self.auth.graphql else {
            return Err(String::new());
        };

        let provider = "graphql";
        let creds = self.render_credentials(profile, &gql.credentials_jq, provider)?;

        let op_path = resolve_path_from_repo_root(repo_root, &gql.login_op);
        if !op_path.is_file() {
            return Err(format!(
                "auth_login_template_render_failed(provider={provider},profile={profile})"
            ));
        }
        let vars_template_path = resolve_path_from_repo_root(repo_root, &gql.login_vars_template);
        if !vars_template_path.is_file() {
            return Err(format!(
                "auth_login_template_render_failed(provider={provider},profile={profile})"
            ));
        }

        let vars_template: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&vars_template_path).map_err(|_| {
                format!("auth_login_template_render_failed(provider={provider},profile={profile})")
            })?)
            .map_err(|_| {
                format!("auth_login_template_render_failed(provider={provider},profile={profile})")
            })?;

        let mut vars_obj = match vars_template {
            serde_json::Value::Object(m) => m,
            _ => {
                return Err(format!(
                    "auth_login_template_render_failed(provider={provider},profile={profile})"
                ))
            }
        };

        let serde_json::Value::Object(creds_obj) = creds else {
            return Err(format!(
                "auth_login_template_render_failed(provider={provider},profile={profile})"
            ));
        };
        for (k, v) in creds_obj {
            vars_obj.insert(k, v);
        }
        let vars_json = serde_json::Value::Object(vars_obj);

        let endpoint_url = resolve_auth_gql_url(
            repo_root,
            &gql.config_dir,
            &gql.url,
            &gql.env,
            suite_defaults,
            env_gql_url,
        )
        .map_err(|_| {
            format!("auth_login_request_failed(provider={provider},profile={profile},rc=1)")
        })?;

        let op_file =
            crate::graphql::schema::GraphqlOperationFile::load(&op_path).map_err(|_| {
                format!("auth_login_template_render_failed(provider={provider},profile={profile})")
            })?;

        let executed = crate::graphql::runner::execute_graphql_request(
            &endpoint_url,
            None,
            &op_file.operation,
            Some(&vars_json),
        )
        .map_err(|_| {
            format!("auth_login_request_failed(provider={provider},profile={profile},rc=1)")
        })?;

        let response_json: serde_json::Value = serde_json::from_slice(&executed.response.body)
            .map_err(|_| format!("auth_token_jq_error(provider={provider},profile={profile})"))?;

        self.extract_token(&response_json, &gql.token_jq, provider, profile)
    }
}

fn canonical_provider(auth: &SuiteAuth) -> Result<AuthProvider> {
    let provider_raw = auth.provider.trim().to_ascii_lowercase();
    let provider = if provider_raw.is_empty() {
        match (&auth.rest, &auth.graphql) {
            (Some(_), None) => "rest".to_string(),
            (None, Some(_)) => "graphql".to_string(),
            (Some(_), Some(_)) => anyhow::bail!(
                "Invalid suite auth block: .auth.provider is required when both .auth.rest and .auth.graphql are present"
            ),
            (None, None) => anyhow::bail!("Invalid suite auth block: missing auth.rest/auth.graphql"),
        }
    } else if provider_raw == "gql" {
        "graphql".to_string()
    } else {
        provider_raw
    };

    match provider.as_str() {
        "rest" => Ok(AuthProvider::Rest),
        "graphql" => Ok(AuthProvider::Graphql),
        _ => {
            anyhow::bail!("Invalid suite auth block: .auth.provider must be one of: rest, graphql")
        }
    }
}

fn inherit_auth_defaults(mut auth: SuiteAuth, suite_defaults: &SuiteDefaults) -> SuiteAuth {
    if let Some(rest) = auth.rest.as_mut() {
        if rest.config_dir.trim().is_empty() {
            rest.config_dir = suite_defaults.rest.config_dir.clone();
        }
    }
    if let Some(gql) = auth.graphql.as_mut() {
        if gql.config_dir.trim().is_empty() {
            gql.config_dir = suite_defaults.graphql.config_dir.clone();
        }
    }
    auth
}

fn resolve_auth_rest_base_url(
    repo_root: &Path,
    config_dir: &str,
    url_override: &str,
    env_override: &str,
    suite_defaults: &SuiteDefaults,
    env_rest_url: &str,
) -> Result<String> {
    let url_override = url_override.trim();
    if !url_override.is_empty() {
        return Ok(url_override.to_string());
    }
    let default_url = suite_defaults.rest.url.trim();
    if !default_url.is_empty() {
        return Ok(default_url.to_string());
    }
    let env_rest_url = env_rest_url.trim();
    if !env_rest_url.is_empty() {
        return Ok(env_rest_url.to_string());
    }

    let env_value = if !env_override.trim().is_empty() {
        env_override.trim()
    } else {
        suite_defaults.env.trim()
    };
    if env_value.is_empty() {
        anyhow::bail!("auth missing rest env/url");
    }

    let setup_dir = resolve_path_from_repo_root(repo_root, config_dir);
    resolve_rest_base_url_for_env(&setup_dir, env_value)
}

fn resolve_auth_gql_url(
    repo_root: &Path,
    config_dir: &str,
    url_override: &str,
    env_override: &str,
    suite_defaults: &SuiteDefaults,
    env_gql_url: &str,
) -> Result<String> {
    let url_override = url_override.trim();
    if !url_override.is_empty() {
        return Ok(url_override.to_string());
    }
    let default_url = suite_defaults.graphql.url.trim();
    if !default_url.is_empty() {
        return Ok(default_url.to_string());
    }
    let env_gql_url = env_gql_url.trim();
    if !env_gql_url.is_empty() {
        return Ok(env_gql_url.to_string());
    }

    let env_value = if !env_override.trim().is_empty() {
        env_override.trim()
    } else {
        suite_defaults.env.trim()
    };
    if env_value.is_empty() {
        anyhow::bail!("auth missing graphql env/url");
    }

    let setup_dir = resolve_path_from_repo_root(repo_root, config_dir);
    resolve_gql_url_for_env(&setup_dir, env_value)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn suite_auth_credentials_jq_requires_exactly_one_object() {
        let auth = SuiteAuth {
            provider: "rest".to_string(),
            required: true,
            secret_env: "API_TEST_AUTH_JSON".to_string(),
            rest: Some(crate::suite::schema::SuiteAuthRest {
                login_request_template: "setup/rest/requests/login.request.json".to_string(),
                credentials_jq: ".profiles[$profile]".to_string(),
                token_jq: ".accessToken".to_string(),
                config_dir: "setup/rest".to_string(),
                url: "http://localhost:0".to_string(),
                env: String::new(),
            }),
            graphql: None,
        };

        // Note: we are testing render_credentials directly; avoid env reads.
        let mgr = SuiteAuthManager {
            provider: AuthProvider::Rest,
            provider_label: "rest".to_string(),
            secret_json: serde_json::json!({"profiles": {"admin": {"u": "a"}}}),
            auth,
            tokens: HashMap::new(),
            errors: HashMap::new(),
        };

        let creds = mgr
            .render_credentials("admin", ".profiles[$profile]", "rest")
            .unwrap();
        assert!(creds.is_object());

        let err = mgr
            .render_credentials("missing", ".profiles[$profile]", "rest")
            .unwrap_err();
        assert!(err.contains("auth_credentials_missing"));
    }

    fn auth_rest_stub() -> crate::suite::schema::SuiteAuthRest {
        crate::suite::schema::SuiteAuthRest {
            login_request_template: "setup/rest/requests/login.request.json".to_string(),
            credentials_jq: ".profiles[$profile]".to_string(),
            token_jq: ".accessToken".to_string(),
            config_dir: "setup/rest".to_string(),
            url: String::new(),
            env: String::new(),
        }
    }

    fn auth_graphql_stub() -> crate::suite::schema::SuiteAuthGraphql {
        crate::suite::schema::SuiteAuthGraphql {
            login_op: "setup/graphql/operations/login.graphql".to_string(),
            login_vars_template: "setup/graphql/vars/login.json".to_string(),
            credentials_jq: ".profiles[$profile]".to_string(),
            token_jq: ".token".to_string(),
            config_dir: "setup/graphql".to_string(),
            url: String::new(),
            env: String::new(),
        }
    }

    #[test]
    fn canonical_provider_infers_rest_when_only_rest_present() {
        let auth = SuiteAuth {
            provider: String::new(),
            required: true,
            secret_env: "API_TEST_AUTH_JSON".to_string(),
            rest: Some(auth_rest_stub()),
            graphql: None,
        };

        let provider = canonical_provider(&auth).expect("provider");
        assert_eq!(provider, AuthProvider::Rest);
    }

    #[test]
    fn canonical_provider_infers_graphql_when_only_graphql_present() {
        let auth = SuiteAuth {
            provider: String::new(),
            required: true,
            secret_env: "API_TEST_AUTH_JSON".to_string(),
            rest: None,
            graphql: Some(auth_graphql_stub()),
        };

        let provider = canonical_provider(&auth).expect("provider");
        assert_eq!(provider, AuthProvider::Graphql);
    }

    #[test]
    fn canonical_provider_supports_gql_alias() {
        let auth = SuiteAuth {
            provider: "gql".to_string(),
            required: true,
            secret_env: "API_TEST_AUTH_JSON".to_string(),
            rest: None,
            graphql: Some(auth_graphql_stub()),
        };

        let provider = canonical_provider(&auth).expect("provider");
        assert_eq!(provider, AuthProvider::Graphql);
    }

    #[test]
    fn canonical_provider_requires_provider_when_both_present() {
        let auth = SuiteAuth {
            provider: String::new(),
            required: true,
            secret_env: "API_TEST_AUTH_JSON".to_string(),
            rest: Some(auth_rest_stub()),
            graphql: Some(auth_graphql_stub()),
        };

        let err = canonical_provider(&auth).unwrap_err().to_string();
        assert!(err.contains(
            ".auth.provider is required when both .auth.rest and .auth.graphql are present"
        ));
    }

    #[test]
    fn canonical_provider_rejects_unknown_provider() {
        let auth = SuiteAuth {
            provider: "nope".to_string(),
            required: true,
            secret_env: "API_TEST_AUTH_JSON".to_string(),
            rest: Some(auth_rest_stub()),
            graphql: None,
        };

        let err = canonical_provider(&auth).unwrap_err().to_string();
        assert!(err.contains(".auth.provider must be one of: rest, graphql"));
    }

    #[test]
    fn init_from_suite_missing_secret_env_required_false_disables_auth() {
        let _g = ENV_LOCK.lock().expect("lock");
        let key = "NILS_TEST_AUTH_JSON_MISSING";
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);

        let auth = SuiteAuth {
            provider: String::new(),
            required: false,
            secret_env: key.to_string(),
            rest: Some(auth_rest_stub()),
            graphql: None,
        };
        let defaults = SuiteDefaults::default();

        let init = SuiteAuthManager::init_from_suite(auth, &defaults).expect("init");
        let AuthInit::Disabled { message } = init else {
            panic!("expected disabled");
        };
        let msg = message.unwrap_or_default();
        assert!(msg.contains("auth disabled"));
        assert!(msg.contains(key));
        assert!(msg.contains("auth.required=false"));

        if let Some(v) = prev {
            std::env::set_var(key, v);
        } else {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn init_from_suite_invalid_json_is_error() {
        let _g = ENV_LOCK.lock().expect("lock");
        let key = "NILS_TEST_AUTH_JSON_INVALID";
        let prev = std::env::var(key).ok();
        std::env::set_var(key, "{");

        let auth = SuiteAuth {
            provider: "rest".to_string(),
            required: true,
            secret_env: key.to_string(),
            rest: Some(auth_rest_stub()),
            graphql: None,
        };
        let defaults = SuiteDefaults::default();

        let err = SuiteAuthManager::init_from_suite(auth, &defaults)
            .unwrap_err()
            .to_string();
        assert!(err.contains(&format!("Invalid JSON in {key}")));

        if let Some(v) = prev {
            std::env::set_var(key, v);
        } else {
            std::env::remove_var(key);
        }
    }

    fn stub_mgr(provider: AuthProvider, provider_label: &str) -> SuiteAuthManager {
        SuiteAuthManager {
            provider,
            provider_label: provider_label.to_string(),
            secret_json: serde_json::Value::Object(serde_json::Map::new()),
            auth: SuiteAuth {
                provider: provider_label.to_string(),
                required: true,
                secret_env: "API_TEST_AUTH_JSON".to_string(),
                rest: None,
                graphql: None,
            },
            tokens: HashMap::new(),
            errors: HashMap::new(),
        }
    }

    #[test]
    fn ensure_token_caches_successful_login_token() {
        use std::cell::Cell;

        let calls = Cell::new(0);
        let mut mgr = stub_mgr(AuthProvider::Rest, "rest");

        let t1 = mgr
            .ensure_token_with_login("admin", |_mgr, profile| {
                calls.set(calls.get() + 1);
                Ok(format!("tok-{profile}"))
            })
            .expect("token");
        assert_eq!(t1, "tok-admin");

        let t2 = mgr
            .ensure_token_with_login("admin", |_mgr, _profile| {
                calls.set(calls.get() + 1);
                Ok("tok-should-not-be-called".to_string())
            })
            .expect("token");
        assert_eq!(t2, "tok-admin");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn ensure_token_memoizes_errors_and_does_not_retry() {
        use std::cell::Cell;

        let calls = Cell::new(0);
        let mut mgr = stub_mgr(AuthProvider::Graphql, "graphql");

        let err1 = mgr
            .ensure_token_with_login("svc", |_mgr, _profile| {
                calls.set(calls.get() + 1);
                Err(String::new())
            })
            .unwrap_err();
        assert_eq!(err1, "auth_login_failed(provider=graphql,profile=svc)");

        let err2 = mgr
            .ensure_token_with_login("svc", |_mgr, _profile| {
                calls.set(calls.get() + 1);
                Ok("tok-should-not-be-called".to_string())
            })
            .unwrap_err();
        assert_eq!(err2, "auth_login_failed(provider=graphql,profile=svc)");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn resolve_auth_rest_base_url_precedence_and_env_lookup() {
        let tmp = TempDir::new().expect("tempdir");
        let repo_root = tmp.path();

        let mut defaults = SuiteDefaults {
            env: "staging".to_string(),
            rest: crate::suite::schema::SuiteDefaultsRest {
                url: "http://default.example".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let got = resolve_auth_rest_base_url(
            repo_root,
            "setup/rest",
            "http://override.example",
            "",
            &defaults,
            "http://env.example",
        )
        .expect("url");
        assert_eq!(got, "http://override.example");

        let got = resolve_auth_rest_base_url(
            repo_root,
            "setup/rest",
            "",
            "",
            &defaults,
            "http://env.example",
        )
        .expect("url");
        assert_eq!(got, "http://default.example");

        defaults.rest.url = String::new();
        let got = resolve_auth_rest_base_url(
            repo_root,
            "setup/rest",
            "",
            "",
            &defaults,
            "http://env.example",
        )
        .expect("url");
        assert_eq!(got, "http://env.example");

        let setup_dir = repo_root.join("setup/rest");
        std::fs::create_dir_all(&setup_dir).expect("mkdir");
        std::fs::write(
            setup_dir.join("endpoints.env"),
            "REST_URL_STAGING=http://staging.example\nREST_URL_PROD=http://prod.example\n",
        )
        .expect("write endpoints.env");

        let got = resolve_auth_rest_base_url(repo_root, "setup/rest", "", "", &defaults, "")
            .expect("url");
        assert_eq!(got, "http://staging.example");

        let got = resolve_auth_rest_base_url(repo_root, "setup/rest", "", "prod", &defaults, "")
            .expect("url");
        assert_eq!(got, "http://prod.example");
    }

    #[test]
    fn resolve_auth_gql_url_precedence_and_env_lookup() {
        let tmp = TempDir::new().expect("tempdir");
        let repo_root = tmp.path();

        let mut defaults = SuiteDefaults {
            env: "staging".to_string(),
            graphql: crate::suite::schema::SuiteDefaultsGraphql {
                url: "http://default.example/graphql".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let got = resolve_auth_gql_url(
            repo_root,
            "setup/graphql",
            "http://override.example/graphql",
            "",
            &defaults,
            "http://env.example/graphql",
        )
        .expect("url");
        assert_eq!(got, "http://override.example/graphql");

        let got = resolve_auth_gql_url(
            repo_root,
            "setup/graphql",
            "",
            "",
            &defaults,
            "http://env.example/graphql",
        )
        .expect("url");
        assert_eq!(got, "http://default.example/graphql");

        defaults.graphql.url = String::new();
        let got = resolve_auth_gql_url(
            repo_root,
            "setup/graphql",
            "",
            "",
            &defaults,
            "http://env.example/graphql",
        )
        .expect("url");
        assert_eq!(got, "http://env.example/graphql");

        let setup_dir = repo_root.join("setup/graphql");
        std::fs::create_dir_all(&setup_dir).expect("mkdir");
        std::fs::write(
            setup_dir.join("endpoints.env"),
            "GQL_URL_STAGING=http://staging.example/graphql\nGQL_URL_PROD=http://prod.example/graphql\n",
        )
        .expect("write endpoints.env");

        let got =
            resolve_auth_gql_url(repo_root, "setup/graphql", "", "", &defaults, "").expect("url");
        assert_eq!(got, "http://staging.example/graphql");

        let got = resolve_auth_gql_url(repo_root, "setup/graphql", "", "prod", &defaults, "")
            .expect("url");
        assert_eq!(got, "http://prod.example/graphql");
    }
}
