use super::*;

pub(super) fn missing_backend(lock: &PeekabooLock, strict: bool) -> DoctorReport {
    let blocked = |id: &str| CheckResult {
        id: id.into(),
        status: "blocked",
        message:
            "backend is not installed; run macos-agent backend install before checking readiness"
                .into(),
    };
    DoctorReport {
        locked_tag: lock.tag.clone(),
        strict,
        ready: false,
        backend: VerificationReport {
            locked_tag: lock.tag.clone(),
            active_tag: String::new(),
            rollback_active: false,
            strict,
            security_posture: match lock.cli_asset().notarization.policy {
                NotarizationPolicy::Required => "full",
                NotarizationPolicy::Waived => "reduced",
            },
            ready: false,
            checks: vec![CheckResult {
                id: "backend".into(),
                status: "fail",
                message:
                    "the locked Peekaboo backend is not installed; run macos-agent backend install"
                        .into(),
            }],
        },
        runtime: blocked("runtime"),
        permissions: blocked("permissions"),
        bridge: blocked("bridge"),
        capabilities: lock
            .required_capability_probes
            .iter()
            .filter(|probe| !matches!(probe.id.as_str(), "permissions" | "bridge"))
            .map(|probe| blocked(&probe.id))
            .collect(),
    }
}

pub fn doctor(strict: bool) -> Result<DoctorReport, CliError> {
    ensure_supported_platform()?;
    let lock = PeekabooLock::embedded()?;
    let paths = BackendPaths::resolve()?;
    reject_symlink_components(&paths.root)?;
    let guard = paths
        .root
        .exists()
        .then(|| LifecycleLock::acquire(&paths.root, LifecycleLockMode::Shared))
        .transpose()?;
    if guard.is_none() || read_receipt(&paths.current_receipt())?.is_none() {
        return Ok(missing_backend(&lock, strict));
    }
    let backend = verify_unlocked(strict, &lock, &paths)?;
    let receipt = read_receipt(&paths.current_receipt())?
        .ok_or_else(|| backend_error("the verified backend receipt is unavailable"))?;
    let (cli_asset, app_asset, _) = release_contract_for_receipt(&lock, &receipt)?;
    let binary = paths.current_cli()?;
    let mut permissions = None;
    let mut bridge = None;
    let mut capabilities = Vec::new();
    let stable_app_socket = crate::commands::stable_app_socket();
    for probe in &lock.required_capability_probes {
        let mut arguments = probe.argv.clone();
        if matches!(probe.id.as_str(), "permissions" | "bridge") {
            arguments.push("--bridge-socket".into());
            arguments.push(stable_app_socket.to_string_lossy().into_owned());
        }
        let argument_refs = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        let output = run_tool(&binary, &argument_refs);
        let (status, message) =
            evaluate_capability_probe(&probe.id, &output, &receipt.tag, &app_asset.bridge_build);
        let message = if probe.id == "permissions" && status != "pass" {
            let value = output
                .as_ref()
                .ok()
                .filter(|output| !output.timed_out && !output.stdout_truncated)
                .and_then(|output| {
                    serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
                });
            let authority = match value
                .as_ref()
                .and_then(|value| value.pointer("/data/source"))
                .and_then(serde_json::Value::as_str)
            {
                Some("local" | "process") => format!("Peekaboo CLI (team {})", cli_asset.team_id),
                _ => format!(
                    "Peekaboo GUI app ({}, team {})",
                    app_asset
                        .bundle_id
                        .as_deref()
                        .unwrap_or("unreported bundle"),
                    app_asset.team_id
                ),
            };
            value.as_ref().and_then(|value| missing_permission_message(value, &authority))
                .unwrap_or_else(|| format!("Screen Recording and Accessibility could not be assessed for {authority}; confirm this authority in System Settings > Privacy & Security, then rerun doctor"))
        } else {
            message.into()
        };
        let check = CheckResult {
            id: probe.id.clone(),
            status,
            message,
        };
        if probe.id == "permissions" {
            permissions = Some(check.clone());
        } else if probe.id == "bridge" {
            bridge = Some(check.clone());
        } else {
            capabilities.push(check);
        }
    }
    let permissions = permissions
        .ok_or_else(|| backend_error("mandatory permissions probe result is unavailable"))?;
    let mut bridge =
        bridge.ok_or_else(|| backend_error("mandatory Bridge probe result is unavailable"))?;
    if bridge.status != "pass" && permissions.status == "blocked" {
        bridge.message = format!("{}; {}", bridge.message, permissions.message);
    }
    let runtime = if paths.stable_app().is_dir() {
        passed("runtime", "stable Peekaboo app path is ready")
    } else {
        CheckResult {
            id: "runtime".into(),
            status: "fail",
            message: "stable Peekaboo app path is unavailable".into(),
        }
    };
    let ready = backend.ready
        && runtime.status == "pass"
        && permissions.status == "pass"
        && bridge.status == "pass"
        && capabilities.iter().all(|check| check.status == "pass");
    Ok(DoctorReport {
        locked_tag: lock.tag,
        strict,
        ready,
        backend,
        runtime,
        permissions,
        bridge,
        capabilities,
    })
}
