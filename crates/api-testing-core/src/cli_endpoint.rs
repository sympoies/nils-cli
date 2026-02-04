use std::path::Path;

use crate::{cli_util, env_file, Result};

#[derive(Debug, Clone)]
pub struct EndpointSelection {
    pub url: String,
    pub endpoint_label_used: String,
    pub endpoint_value_used: String,
}

pub struct EndpointConfig<'a> {
    pub explicit_url: Option<&'a str>,
    pub env_name: Option<&'a str>,
    pub endpoints_env: &'a Path,
    pub endpoints_local: &'a Path,
    pub endpoints_files: &'a [&'a Path],
    pub url_env_var: &'a str,
    pub env_default_var: &'a str,
    pub url_prefix: &'a str,
    pub default_url: &'a str,
    pub setup_dir_label: &'a str,
}

pub fn resolve_cli_endpoint(cfg: EndpointConfig<'_>) -> Result<EndpointSelection> {
    let env_default = if !cfg.endpoints_files.is_empty() {
        env_file::read_var_last_wins(cfg.env_default_var, cfg.endpoints_files)?
    } else {
        None
    };

    let explicit_url = cfg.explicit_url.and_then(cli_util::trim_non_empty);
    let env_name = cfg.env_name.and_then(cli_util::trim_non_empty);

    let (url, endpoint_label_used, endpoint_value_used) = if let Some(url) = explicit_url {
        (url.clone(), "url".to_string(), url)
    } else if let Some(env_value) = env_name.as_deref() {
        if env_value.starts_with("http://") || env_value.starts_with("https://") {
            (
                env_value.to_string(),
                "url".to_string(),
                env_value.to_string(),
            )
        } else {
            if cfg.endpoints_files.is_empty() {
                anyhow::bail!(
                    "endpoints.env not found (expected under {setup_dir_label})",
                    setup_dir_label = cfg.setup_dir_label
                );
            }

            let env_key = cli_util::to_env_key(env_value);
            let key = format!("{url_prefix}{env_key}", url_prefix = cfg.url_prefix);
            let found = env_file::read_var_last_wins(&key, cfg.endpoints_files)?;
            let Some(found) = found else {
                let mut available =
                    cli_util::list_available_suffixes(cfg.endpoints_env, cfg.url_prefix);
                if cfg.endpoints_local.is_file() {
                    available.extend(cli_util::list_available_suffixes(
                        cfg.endpoints_local,
                        cfg.url_prefix,
                    ));
                    available.sort();
                    available.dedup();
                }
                let available = if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(" ")
                };
                anyhow::bail!("Unknown --env '{env_value}' (available: {available})");
            };

            (found, "env".to_string(), env_value.to_string())
        }
    } else if let Some(v) = std::env::var(cfg.url_env_var)
        .ok()
        .and_then(|s| cli_util::trim_non_empty(&s))
    {
        (v.clone(), "url".to_string(), v)
    } else if let Some(default_env) = env_default {
        if cfg.endpoints_files.is_empty() {
            anyhow::bail!(
                "{env_default_var} is set but endpoints.env not found (expected under {setup_dir_label})",
                env_default_var = cfg.env_default_var,
                setup_dir_label = cfg.setup_dir_label
            );
        }
        let env_key = cli_util::to_env_key(&default_env);
        let key = format!("{url_prefix}{env_key}", url_prefix = cfg.url_prefix);
        let found = env_file::read_var_last_wins(&key, cfg.endpoints_files)?;
        let Some(found) = found else {
            anyhow::bail!(
                "{env_default_var} is '{}' but no matching {url_prefix}* was found.",
                default_env,
                env_default_var = cfg.env_default_var,
                url_prefix = cfg.url_prefix
            );
        };
        (found, "env".to_string(), default_env)
    } else {
        let url = cfg.default_url.to_string();
        (url.clone(), "url".to_string(), url)
    };

    Ok(EndpointSelection {
        url,
        endpoint_label_used,
        endpoint_value_used,
    })
}

pub fn list_available_env_suffixes(
    endpoints_env: &Path,
    endpoints_local: &Path,
    url_prefix: &str,
    missing_message: &str,
) -> Result<Vec<String>> {
    if !endpoints_env.is_file() && !endpoints_local.is_file() {
        anyhow::bail!("{missing_message}");
    }

    let mut out = Vec::new();
    if endpoints_env.is_file() {
        out.extend(cli_util::list_available_suffixes(endpoints_env, url_prefix));
    }
    if endpoints_local.is_file() {
        out.extend(cli_util::list_available_suffixes(
            endpoints_local,
            url_prefix,
        ));
    }
    out.sort();
    out.dedup();

    Ok(out)
}
