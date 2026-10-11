use std::io;

use nils_common::fs::write_atomic;

use super::*;

pub(super) fn read_receipt(path: &Path) -> Result<Option<Receipt>, CliError> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(backend_error("failed to read a backend receipt")),
    };
    let receipt = serde_json::from_slice::<Receipt>(&raw)
        .map_err(|_| backend_error("backend receipt is malformed"))?;
    if receipt.schema_version != RECEIPT_SCHEMA
        || !safe_tag(&receipt.tag)
        || !receipt_digests_are_valid(&receipt)
    {
        return Err(backend_error("backend receipt is unsupported or unsafe"));
    }
    Ok(Some(receipt))
}

fn receipt_digests_are_valid(receipt: &Receipt) -> bool {
    [
        &receipt.cli_archive_sha256,
        &receipt.app_archive_sha256,
        &receipt.cli_binary_sha256,
        &receipt.app_binary_sha256,
    ]
    .into_iter()
    .all(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

pub(super) fn write_receipt(path: &Path, receipt: &Receipt) -> Result<(), CliError> {
    let body = serde_json::to_vec_pretty(receipt)
        .map_err(|_| backend_error("failed to encode backend receipt"))?;
    write_atomic(path, &body, SECRET_FILE_MODE)
        .map_err(|_| backend_error("failed to atomically write backend receipt"))
}
