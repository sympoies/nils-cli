//! Recover accepted reply obligations before source journal compaction.
use super::*;

pub(super) fn recover(
    registry: &mut super::super::LockedRegistry,
    journal: &Journal,
) -> Result<(), CliError> {
    let mut changed = false;
    for reply in journal
        .remote_outbox
        .iter()
        .filter(|reply| reply.clears_reply_obligation)
    {
        changed |= super::super::mailbox::record_reply(
            &mut registry.registry,
            reply.envelope.reply_to.as_deref(),
            reply.envelope.created_at_epoch,
        );
    }
    if changed {
        registry.save()?;
    }
    Ok(())
}

pub(super) fn lock_recovered(context: &CliContext) -> Result<LockedJournal, CliError> {
    let journal = lock_journal(context)?;
    if !journal
        .registry
        .remote_outbox
        .iter()
        .any(|reply| reply.clears_reply_obligation)
    {
        // Ordinary outboxes retain their independence from the mailbox registry.
        return Ok(journal);
    }
    // Drop the peek before acquiring registry -> journal, never the inverse.
    drop(journal);
    let mut registry = lock_registry(context)?;
    let journal = lock_journal(context)?;
    recover(&mut registry, &journal.registry)?;
    // Returning the held journal guard prevents new acceptance from racing the
    // caller's compaction after recovery persisted the original markers.
    Ok(journal)
}

pub(super) fn replay(context: &CliContext, parent: Option<&str>) -> Result<(), CliError> {
    if parent.is_some() {
        let mut registry = lock_registry(context)?;
        if super::super::mailbox::record_reply(&mut registry.registry, parent, now_epoch()) {
            registry.save()?;
        }
    }
    Ok(())
}
