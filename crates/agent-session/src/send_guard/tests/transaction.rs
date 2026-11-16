//! Run the unchanged native composer assertions through the real transaction,
//! with fixture budgets independent of the shipped observation/hold caps.
use clap::Parser;
use nils_common::cli_contract::{Envelope, EnvelopeError, schema_version_for};
use pretty_assertions::assert_eq;
use std::time::Duration;

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/guarded_send/fixture.rs"
));

#[path = "../../../tests/guarded_send/native_composer.rs"]
mod native_composer;

struct Budget {
    _scope: crate::send::test_budget::Scope,
    retry: Duration,
}

impl Budget {
    fn enter() -> Self {
        Self {
            // The model still performs every real subprocess/state transition.
            // Give the fixture scheduling room without changing production caps.
            _scope: crate::send::test_budget::Scope::enter(Duration::from_secs(120)),
            retry: super::super::TEST_RETRY_WINDOW.replace(Duration::ZERO),
        }
    }
}

impl Drop for Budget {
    fn drop(&mut self) {
        super::super::TEST_RETRY_WINDOW.set(self.retry);
    }
}

fn run_fixture(root: &Path, args: &[&str]) -> CmdOutput {
    let _budget = Budget::enter();
    let cli = crate::cli::Cli::try_parse_from(
        std::iter::once("agent-session").chain(args.iter().copied()),
    )
    .unwrap();
    let crate::cli::Command::Send(args) = cli.command else {
        panic!("transaction fixture requires send arguments");
    };
    let context = crate::CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    let schema = schema_version_for(crate::SCHEMA, crate::SEND_COMMAND, 1);
    let (code, value) = match crate::send::send_to_session(&context, args) {
        Ok(result) => (
            0,
            serde_json::to_value(Envelope::success(schema, result)).unwrap(),
        ),
        Err(error) => {
            let error = error.into_inner();
            let envelope: Envelope<()> = Envelope::failure(
                schema,
                EnvelopeError {
                    code: error.code,
                    message: error.message,
                    details: error.details,
                    hint: error.hint,
                },
            );
            (error.exit_code, serde_json::to_value(envelope).unwrap())
        }
    };
    CmdOutput {
        code,
        stdout: serde_json::to_vec(&value).unwrap(),
        stderr: Vec::new(),
    }
}
