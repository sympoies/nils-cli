//! Composer bounds resolved before serve starts threads.
use super::*;

pub(super) fn resolve(
    key: &'static str,
    environment_key: &'static str,
    parse: fn(&str) -> Result<u64, CliError>,
    value: Option<&Value>,
    lookup: &dyn Fn(&str) -> Option<OsString>,
    warnings: &mut Vec<String>,
    assignments: &mut Vec<(&'static str, OsString)>,
) -> Result<Source, CliError> {
    let file = value
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| invalid_value(key, "invalid composer bound in seconds"))
        })
        .transpose()?;
    if let Some(seconds) = file {
        parse(&seconds.to_string())
            .map_err(|_| invalid_value(key, "invalid composer bound in seconds"))?;
    }
    let environment = lookup(environment_key)
        .map(|value| {
            value.into_string().map_err(|_| {
                keyed_error(
                    "serve-config-invalid-value",
                    key,
                    "environment",
                    "must be unsigned integer seconds",
                )
            })
        })
        .transpose()?;
    if let Some(raw) = environment {
        parse(&raw).map_err(|_| {
            keyed_error(
                "serve-config-invalid-value",
                key,
                "environment",
                "must be unsigned integer seconds",
            )
        })?;
        if file.is_some() {
            warnings.push(format!(
                "{environment_key} overrides the config file's {key}"
            ));
        }
        return Ok(Source::Environment);
    }
    if let Some(seconds) = file {
        assignments.push((environment_key, OsString::from(seconds.to_string())));
        return Ok(Source::File);
    }
    Ok(Source::None)
}
