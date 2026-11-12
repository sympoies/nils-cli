//! One history projection owns a lazy snapshot of upload ownership proofs.
use super::*;
use std::collections::HashMap;

pub(super) struct UploadOwnership<'a> {
    pub(super) catalog: &'a HistoryCatalog,
    pub(super) session: &'a HistorySession,
    proof: Option<Result<OwnershipProof, ()>>,
}

struct OwnershipProof {
    archived: Option<String>,
    context: crate::CliContext,
    live: HashMap<String, bool>,
}

impl<'a> UploadOwnership<'a> {
    pub(super) fn new(catalog: &'a HistoryCatalog, session: &'a HistorySession) -> Self {
        Self {
            catalog,
            session,
            proof: None,
        }
    }

    pub(super) fn attachment_root(
        &mut self,
        path: &Path,
    ) -> Result<(PathBuf, PathBuf), ImageReadError> {
        let state = self
            .catalog
            .archives_root
            .parent()
            .and_then(Path::parent)
            .ok_or(ImageReadError::Unavailable)?;
        let relative = path
            .strip_prefix(state)
            .map_err(|_| ImageReadError::Unavailable)?;
        let parts: Vec<_> = relative.components().collect();
        if parts.len() != 4
            || parts[0].as_os_str() != "sessions"
            || parts[2].as_os_str() != "attachments"
        {
            return Err(ImageReadError::Unavailable);
        }
        let id = parts[1]
            .as_os_str()
            .to_str()
            .ok_or(ImageReadError::Unavailable)?;
        // Preserve the existing proof and loaders. Cache both grants and refusals
        // per managed session, and discard them when this history read ends.
        let proof = self
            .proof
            .get_or_insert_with(|| {
                let archived = read_archive(&self.catalog.archives_root, &self.session.id)
                    .and_then(|a| a.session_id);
                let context =
                    crate::CliContext::resolve(Some(state.to_path_buf()), None).map_err(|_| ())?;
                Ok(OwnershipProof {
                    archived,
                    context,
                    live: HashMap::new(),
                })
            })
            .as_mut()
            .map_err(|_| ImageReadError::Unavailable)?;
        let archived = proof.archived.as_deref() == Some(id);
        let live = *proof.live.entry(id.to_owned()).or_insert_with(|| {
            crate::load_session_record(&proof.context, id)
                .ok()
                .is_some_and(|r| {
                    r.provider_resume.is_some_and(|p| {
                        p.provider == self.session.provider
                            && p.session_id == self.session.provider_session_id
                    })
                })
        });
        if !archived && !live {
            return Err(ImageReadError::Deleted);
        }
        Ok((state.to_path_buf(), relative.to_path_buf()))
    }

    pub(super) fn owned_original_name(&mut self, path: &Path) -> Option<String> {
        let (root, relative) = self.attachment_root(path).ok()?;
        HistoryCatalog::managed_original_name(&root, &relative)
    }
}
