//! Explicit, literal-input workflow dispatch.
use super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, BackendProgram};
use crate::cli::{GlobalFlags, WorkflowCommand};
use crate::error::ForgeError;
use nils_common::cli_contract::OutputFormat;
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
#[derive(Serialize)]
struct DispatchPayload {
    provider: &'static str,
    repository: String,
    workflow: String,
    git_ref: String,
    dispatched: bool,
}
pub fn run(
    global: &GlobalFlags,
    command: WorkflowCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let WorkflowCommand::Dispatch(args) = command;
    let op = "workflow.dispatch";
    let ctx = write::target(global, op)?;
    write::selector(&args.workflow, op)?;
    write::selector(&args.git_ref, op)?;
    if !args.workflow.bytes().all(|b| b.is_ascii_digit())
        && (!args.workflow.ends_with(".yml") && !args.workflow.ends_with(".yaml")
            || args.workflow.contains('/'))
    {
        return Err(invalid(
            op,
            "workflow_selector_invalid",
            "supply a numeric workflow id or a workflow file name ending in .yml or .yaml",
        ));
    }
    let source_inputs = match args.inputs_file {
        Some(path) => {
            let json = write::text_file(&path, op)?;
            serde_json::from_str::<UniqueInputs>(&json)
                .map_err(|_| invalid(op, "workflow_inputs_file_invalid", "workflow inputs file must be a JSON object with unique keys and string values"))?
                .0.into_iter().collect::<Vec<_>>()
        }
        None => args
            .inputs
            .into_iter()
            .map(|input| {
                input
                    .split_once('=')
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .ok_or_else(|| {
                        invalid(
                            op,
                            "workflow_input_invalid",
                            "workflow inputs must be KEY=VALUE",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let mut inputs = BTreeMap::new();
    for (key, value) in source_inputs {
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid(
                op,
                "workflow_input_invalid",
                "workflow input keys must contain letters, digits, '_' or '-'",
            ));
        }
        if inputs.insert(key, value).is_some() {
            return Err(invalid(
                op,
                "workflow_input_duplicate",
                "workflow input keys must be unique",
            ));
        }
    }
    let payload = serde_json::json!({"ref": args.git_ref, "inputs": inputs}).to_string();
    let payload_file = write::payload_file(payload.as_bytes(), op)?;
    let workflow: String = url::form_urlencoded::byte_serialize(args.workflow.as_bytes()).collect();
    let repo = ctx.repo.as_deref().expect("target repository");
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        format!("repos/{repo}/actions/workflows/{workflow}/dispatches").into(),
        "--method".into(),
        "POST".into(),
        "--input".into(),
        payload_file.path().as_os_str().to_owned(),
    ]);
    let result = write::execute(
        global,
        op,
        &ctx,
        BackendCall::new(BackendProgram::Gh, argv),
        format,
        |_| {
            Ok(DispatchPayload {
                provider: "github",
                repository: ctx.repo.clone().expect("target repository"),
                workflow: args.workflow,
                git_ref: args.git_ref,
                dispatched: true,
            })
        },
    );
    drop(payload_file);
    result
}

/// Preserve the same duplicate-key refusal for inline and file input forms.
struct UniqueInputs(BTreeMap<String, String>);
impl<'de> serde::Deserialize<'de> for UniqueInputs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueInputs;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object with unique keys and string values")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut inputs = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if inputs.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate input key"));
                    }
                }
                Ok(UniqueInputs(inputs))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
