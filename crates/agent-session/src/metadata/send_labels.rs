use serde::Serialize;
use serde_json::{Map, Value};

use super::{LABEL_MAX_BYTES, VALUE_MAX_BYTES, read_exact_record, write_exact_record};
use crate::{
    CliContext, CliError, SessionRecord, acquire_session_record_lock, cli::MetadataSetLabelArgs,
    render_error, render_single_success, validate_id,
};

const FIELD: &str = "send_labels";
const MAX_LABELS: usize = 32;
const SET_COMMAND: &str = "metadata-set-label";
const EXIT_LABEL_MISMATCH: i32 = 75;

#[derive(Serialize)]
pub(crate) struct Label<'a> {
    key: &'a str,
    value: &'a str,
}

impl<'a> Label<'a> {
    pub(crate) fn parse(input: &'a str) -> Result<Self, CliError> {
        let (key, value) = input.split_once('=').ok_or_else(invalid)?;
        if key.is_empty()
            || key.len() > LABEL_MAX_BYTES
            || !key.as_bytes()[0].is_ascii_lowercase()
            || !key.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            })
            || value.is_empty()
            || value.len() > VALUE_MAX_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(invalid());
        }
        Ok(Self { key, value })
    }

    /// The caller holds the destination record lock until input or enqueue commits.
    pub(crate) fn check(&self, record: &SessionRecord) -> Result<(), CliError> {
        let actual = record
            .extra
            .get(FIELD)
            .and_then(|labels| labels.get(self.key))
            .and_then(Value::as_str);
        if actual != Some(self.value) {
            return Err(CliError::with_exit_code(
                "send-label-mismatch",
                "destination label is missing or does not match the expected value",
                None,
                EXIT_LABEL_MISMATCH,
            ));
        }
        Ok(())
    }
}

fn invalid() -> CliError {
    CliError::usage(
        "send-label-invalid",
        "expected KEY=VALUE with a lowercase ASCII key and a nonempty value of at most 256 bytes without control characters",
        None,
    )
}

pub(super) fn run_set(context: &CliContext, args: MetadataSetLabelArgs) -> i32 {
    #[derive(Serialize)]
    struct SetResult<'a> {
        id: &'a str,
        key: &'a str,
    }

    let result = Label::parse(&args.label).and_then(|label| {
        set(context, &args.id, &label)?;
        Ok(SetResult {
            id: &args.id,
            key: label.key,
        })
    });
    match result {
        Ok(result) => render_single_success(SET_COMMAND, args.format, &result, |stored| {
            format!("set label {} for {}\n", stored.key, stored.id)
        }),
        Err(error) => render_error(SET_COMMAND, args.format, error),
    }
}

pub(crate) fn set(context: &CliContext, id: &str, label: &Label<'_>) -> Result<(), CliError> {
    validate_id(id)?;
    let _lock = acquire_session_record_lock(context, id)?;
    let mut record = read_exact_record(context, id)?;
    let labels = record
        .extra
        .entry(FIELD.into())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            CliError::data(
                "send-label-state-invalid",
                "stored delivery labels are invalid",
                None,
            )
        })?;
    if labels.len() >= MAX_LABELS && !labels.contains_key(label.key) {
        return Err(CliError::data(
            "send-label-capacity-exceeded",
            "session delivery label capacity is exhausted",
            None,
        ));
    }
    labels.insert(label.key.into(), Value::String(label.value.into()));
    write_exact_record(context, &record)
}
