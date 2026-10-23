use super::{LinuxProcessIdentity, SessionTerminationFailure};
use std::{fs, io};

pub(super) fn read_linux_process_identity(
    pid: libc::pid_t,
) -> Result<Option<LinuxProcessIdentity>, SessionTerminationFailure> {
    identity_from_stat(pid, fs::read_to_string(format!("/proc/{pid}/stat")))
}

fn identity_from_stat(
    pid: libc::pid_t,
    stat: io::Result<String>,
) -> Result<Option<LinuxProcessIdentity>, SessionTerminationFailure> {
    let stat = match stat {
        Ok(stat) => stat,
        // procfs can open the stat inode before exit, then return ESRCH when
        // reading it after the task is reaped. Both errors prove absence.
        Err(err)
            if err.kind() == io::ErrorKind::NotFound || err.raw_os_error() == Some(libc::ESRCH) =>
        {
            return Ok(None);
        }
        Err(_) => return Err(SessionTerminationFailure::VerificationFailed),
    };
    let fields: Vec<&str> = stat
        .rsplit_once(") ")
        .map(|(_, fields)| fields.split_whitespace().collect())
        .ok_or(SessionTerminationFailure::VerificationFailed)?;
    if fields.len() <= 19 {
        return Err(SessionTerminationFailure::VerificationFailed);
    }
    let process_group_id = fields[2]
        .parse()
        .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
    let session_id = fields[3]
        .parse()
        .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
    let start_time = fields[19]
        .parse()
        .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
    Ok(Some(LinuxProcessIdentity {
        pid,
        process_group_id,
        session_id,
        start_time,
        zombie: fields[0] == "Z",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    #[test]
    fn reaped_process_stat_descriptor_is_absent_not_unverified() {
        let mut child = Command::new("sh")
            .args(["-c", "read line"])
            .stdin(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        let mut stat = fs::File::open(format!("/proc/{pid}/stat")).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();

        let mut text = String::new();
        let read = stat.read_to_string(&mut text).map(|_| text);
        assert_eq!(read.as_ref().unwrap_err().raw_os_error(), Some(libc::ESRCH));
        assert!(identity_from_stat(pid, read).unwrap().is_none());
        assert!(read_linux_process_identity(pid).unwrap().is_none());
        assert!(identity_from_stat(pid, Err(io::ErrorKind::PermissionDenied.into())).is_err());
    }
}
