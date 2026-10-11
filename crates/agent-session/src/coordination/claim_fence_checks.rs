use super::*;

/// Read-only operator preview: even an inactive retained fence must be resolved
/// by its owner before a retirement can release the protected claim tuple.
pub(super) fn claim_mutation_fence_absent(
    context: &CliContext,
    session_id: &str,
    incarnation: &str,
    claim: &context::WorkContextRecord,
) -> Result<bool, CliError> {
    let digest = digest_bytes(
        format!(
            "{session_id}\0{incarnation}\0{}\0{}\0{}",
            claim.claim_id, claim.revision, claim.expires_at_epoch
        )
        .as_bytes(),
    );
    let directory = context
        .state_dir
        .join("coordination")
        .join(CLAIM_MUTATION_FENCES_DIR);
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0 => {}
        _ => return Err(store_untrusted()),
    }
    let path = directory.join(digest.trim_start_matches("sha256:"));
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err(store_unavailable()),
        Ok(_) => Ok(false),
    }
}

pub(crate) fn ensure_claim_mutation_not_fenced(
    context: &CliContext,
    session_id: &str,
    session_incarnation: &str,
    claim: &context::WorkContextRecord,
) -> Result<(), CliError> {
    let claim = ControllerClaimTuple {
        claim_id: claim.claim_id.clone(),
        revision: claim.revision,
        expires_at_epoch: claim.expires_at_epoch,
    };
    let path = claim_mutation_fence_sidecar_path(context, session_id, session_incarnation, &claim)?;
    let fence = match read_claim_mutation_fence(&path) {
        Ok(fence) => fence,
        Err(error) if error.code() == "claim-mutation-fence-conflict" => return Ok(()),
        Err(error) => return Err(error),
    };
    if !claim_fence_binds_tuple(&fence, session_id, session_incarnation, &claim) {
        return Err(store_corrupt());
    }
    if claim_mutation_fence_owner_active(context, &fence)? {
        return Err(CliError::unavailable(
            "claim-mutation-fenced",
            "the exact claim tuple is fenced by an in-flight claimed runtime stop",
            None,
        ));
    }
    remove_inactive_claim_mutation_fence_sidecar(context, &path, &fence)
}
