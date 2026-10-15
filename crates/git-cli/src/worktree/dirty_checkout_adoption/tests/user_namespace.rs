#[cfg(target_os = "linux")]
use super::*;

#[test]
#[cfg(target_os = "linux")]
fn user_namespace_overflow_owner_requires_unmapped_host_root() {
    let overflow_uid = 65_534;

    assert!(!linux_overflow_uid_proves_unmapped_host_root(
        overflow_uid,
        overflow_uid,
        "0 0 4294967295\n"
    ));
    assert!(linux_overflow_uid_proves_unmapped_host_root(
        overflow_uid,
        overflow_uid,
        "0 1000 1\n1 100000 65533\n"
    ));
    assert!(!linux_overflow_uid_proves_unmapped_host_root(
        1_000,
        overflow_uid,
        "0 1000 1\n"
    ));
}

#[test]
#[cfg(target_os = "linux")]
fn user_namespace_overflow_owner_rejects_malformed_or_ambiguous_maps() {
    let overflow_uid = 65_534;
    for uid_map in [
        "",
        "0 1000\n",
        "0 1000 0\n",
        "0 1000 1 trailing\n",
        "0 nope 1\n",
        "0 +1000 1\n",
        "65534 1000 1\n",
        "65000 1000 1000\n",
        "0 1000 18446744073709551615\n",
    ] {
        assert!(
            !linux_overflow_uid_proves_unmapped_host_root(overflow_uid, overflow_uid, uid_map),
            "malformed or ambiguous UID maps must fail closed"
        );
    }
}
