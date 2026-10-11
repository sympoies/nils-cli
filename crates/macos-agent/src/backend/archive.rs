use super::*;

pub(super) fn download_asset(asset: &AssetLock, downloads: &Path) -> Result<PathBuf, CliError> {
    let destination = downloads.join(&asset.name);
    if test_mode::enabled()
        && let Some(fixtures) = std::env::var_os("NILS_MACOS_AGENT_TEST_ASSET_DIR")
    {
        let fixture = PathBuf::from(fixtures).join(&asset.name);
        fs::copy(&fixture, &destination)
            .map_err(|_| backend_error(format!("test asset `{}` is unavailable", asset.name)))?;
        return Ok(destination);
    }
    let args = vec![
        "--fail".into(),
        "--location".into(),
        "--silent".into(),
        "--show-error".into(),
        "--proto".into(),
        "=https".into(),
        "--tlsv1.2".into(),
        "--output".into(),
        destination.to_string_lossy().into_owned(),
        asset.url.clone(),
    ];
    let output = process::run(
        &backend_tool("curl")?,
        &args,
        &[],
        &[],
        None,
        Duration::from_secs(300),
    )
    .map_err(|_| backend_error("failed to start the HTTPS downloader"))?;
    if output.timed_out || output.exit_code != 0 {
        return Err(backend_error(format!(
            "failed to download locked asset `{}`",
            asset.name
        )));
    }
    Ok(destination)
}

pub(super) fn verify_archive_digest(path: &Path, asset: &AssetLock) -> Result<(), CliError> {
    if hash_file(path)? != asset.sha256 {
        return Err(backend_error(format!(
            "SHA256 mismatch for locked asset `{}`",
            asset.name
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ArchiveKind {
    TarGz,
    Zip,
}

pub(super) fn validate_archive_listing(path: &Path, kind: ArchiveKind) -> Result<(), CliError> {
    let (program, args) = match kind {
        ArchiveKind::TarGz => (
            backend_tool("tar")?,
            vec!["-tzf".into(), path.to_string_lossy().into_owned()],
        ),
        ArchiveKind::Zip => (
            backend_tool("unzip")?,
            vec!["-Z1".into(), path.to_string_lossy().into_owned()],
        ),
    };
    let output = process::run(&program, &args, &[], &[], None, Duration::from_secs(30))
        .map_err(|_| backend_error("failed to inspect a locked archive"))?;
    if output.exit_code != 0 || output.timed_out || output.stdout_truncated {
        return Err(backend_error(
            "locked archive listing failed or exceeded its bound",
        ));
    }
    let listing = String::from_utf8(output.stdout)
        .map_err(|_| backend_error("locked archive listing is not UTF-8"))?;
    let mut count = 0usize;
    for raw in listing.lines() {
        count += 1;
        if count > 20_000 {
            return Err(backend_error("locked archive contains too many entries"));
        }
        validate_archive_path(raw)?;
    }
    if count == 0 {
        return Err(backend_error("locked archive is empty"));
    }
    Ok(())
}

pub(super) fn validate_archive_path(raw: &str) -> Result<(), CliError> {
    let path = Path::new(raw);
    if raw.is_empty()
        || path.is_absolute()
        || raw.contains('\0')
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(backend_error("locked archive contains an unsafe path"));
    }
    Ok(())
}

pub(super) fn extract_archive(
    path: &Path,
    destination: &Path,
    kind: ArchiveKind,
) -> Result<(), CliError> {
    let (program, args) = match kind {
        ArchiveKind::TarGz => (
            backend_tool("tar")?,
            vec![
                "-xzf".into(),
                path.to_string_lossy().into_owned(),
                "-C".into(),
                destination.to_string_lossy().into_owned(),
            ],
        ),
        ArchiveKind::Zip => (
            backend_tool("unzip")?,
            vec![
                "-q".into(),
                path.to_string_lossy().into_owned(),
                "-d".into(),
                destination.to_string_lossy().into_owned(),
            ],
        ),
    };
    let output = process::run(&program, &args, &[], &[], None, Duration::from_secs(120))
        .map_err(|_| backend_error("failed to start archive extraction"))?;
    if output.exit_code != 0 || output.timed_out {
        return Err(backend_error("locked archive extraction failed"));
    }
    Ok(())
}

pub(super) fn validate_symlink_tree(root: &Path) -> Result<(), CliError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|_| backend_error("failed to resolve archive staging root"))?;
    visit_tree(root, &mut |path, file_type| {
        if !file_type.is_symlink() {
            return Ok(());
        }
        let target =
            fs::read_link(path).map_err(|_| backend_error("failed to inspect archive symlink"))?;
        if target.is_absolute() {
            return Err(backend_error("locked archive contains an absolute symlink"));
        }
        let resolved = path
            .canonicalize()
            .map_err(|_| backend_error("locked archive contains a broken symlink"))?;
        if !resolved.starts_with(&canonical_root) {
            return Err(backend_error(
                "locked archive symlink escapes the staging root",
            ));
        }
        Ok(())
    })
}

pub(super) fn visit_tree(
    path: &Path,
    visitor: &mut impl FnMut(&Path, fs::FileType) -> Result<(), CliError>,
) -> Result<(), CliError> {
    for entry in
        fs::read_dir(path).map_err(|_| backend_error("failed to inspect extracted archive"))?
    {
        let entry =
            entry.map_err(|_| backend_error("failed to inspect extracted archive entry"))?;
        let file_type = entry
            .file_type()
            .map_err(|_| backend_error("failed to classify extracted archive entry"))?;
        let entry_path = entry.path();
        visitor(&entry_path, file_type)?;
        if file_type.is_dir() {
            visit_tree(&entry_path, visitor)?;
        }
    }
    Ok(())
}

pub(super) fn verify_version(binary: &Path, tag: &str) -> Result<(), CliError> {
    let output = run_tool(binary, &["--version"])?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.exit_code != 0 || output.timed_out || !text.contains(tag.trim_start_matches('v')) {
        return Err(backend_error(
            "Peekaboo version probe does not match the receipt",
        ));
    }
    Ok(())
}

pub(super) fn verify_architectures(binary: &Path, expected: &[String]) -> Result<(), CliError> {
    let args = ["-archs".into(), binary.to_string_lossy().into_owned()];
    let output = run_tool(
        Path::new("lipo"),
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let actual = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected_set = expected.iter().cloned().collect::<BTreeSet<_>>();
    if output.exit_code != 0
        || output.timed_out
        || output.stdout_truncated
        || actual.is_empty()
        || actual_set.len() != actual.len()
        || expected_set.is_empty()
        || expected_set.len() != expected.len()
        || actual_set != expected_set
    {
        return Err(backend_error(
            "Peekaboo executable architecture does not match the lock",
        ));
    }
    Ok(())
}

pub(super) fn verify_app_metadata(
    app: &Path,
    asset: &AssetLock,
    tag: &str,
    minimum_macos: &str,
) -> Result<(), CliError> {
    let raw = fs::read_to_string(app.join("Contents/Info.plist"))
        .map_err(|_| backend_error("Peekaboo app metadata is unavailable"))?;
    let document = roxmltree::Document::parse_with_options(
        &raw,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 16_384,
            entity_resolver: None,
        },
    )
    .map_err(|_| backend_error("Peekaboo app metadata is malformed"))?;
    let dictionary = document
        .descendants()
        .find(|node| node.has_tag_name("dict"))
        .ok_or_else(|| backend_error("Peekaboo app metadata dictionary is unavailable"))?;
    let bundle_build = bridge_build_number(&asset.bridge_build, tag)
        .ok_or_else(|| backend_error("locked Peekaboo app build identity is malformed"))?;
    for (key, expected) in [
        (
            "CFBundleIdentifier",
            asset.bundle_id.as_deref().unwrap_or_default(),
        ),
        ("CFBundleShortVersionString", tag.trim_start_matches('v')),
        ("CFBundleVersion", bundle_build),
        ("LSMinimumSystemVersion", minimum_macos),
    ] {
        if expected.is_empty() || plist_string(dictionary, key).as_deref() != Some(expected) {
            return Err(backend_error(
                "Peekaboo app metadata does not match the lock",
            ));
        }
    }
    Ok(())
}

pub(super) fn plist_string(dictionary: roxmltree::Node<'_, '_>, key: &str) -> Option<String> {
    let elements = dictionary
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let mut matches = elements.as_chunks::<2>().0.iter().filter_map(|pair| {
        (pair[0].has_tag_name("key")
            && pair[0].text() == Some(key)
            && pair[1].has_tag_name("string"))
        .then(|| pair[1].text().map(str::to_string))
        .flatten()
    });
    let value = matches.next()?;
    matches.next().is_none().then_some(value)
}

pub(super) fn verify_signature(
    path: &Path,
    asset: &AssetLock,
    app: bool,
    require_cli_assessment: bool,
) -> Result<NotarizationAssessment, CliError> {
    let path_text = path.to_string_lossy().into_owned();
    let mut verify_args = vec!["--verify", "--deep", "--strict", "--verbose=2"];
    verify_args.push(&path_text);
    let verify = run_tool(Path::new("codesign"), &verify_args)?;
    if verify.exit_code != 0 || verify.timed_out {
        return Err(backend_error("Peekaboo code signature verification failed"));
    }
    let metadata_args = vec!["-dv", "--verbose=4", &path_text];
    let metadata = run_tool(Path::new("codesign"), &metadata_args)?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&metadata.stdout),
        String::from_utf8_lossy(&metadata.stderr)
    );
    if metadata.exit_code != 0
        || !text.contains(&asset.signing_authority)
        || !text.contains(&format!("TeamIdentifier={}", asset.team_id))
    {
        return Err(backend_error(
            "Peekaboo signing identity does not match the lock",
        ));
    }
    if app {
        let assessment = run_tool(Path::new("spctl"), &["-a", "-vv", &path_text])?;
        let assessment_text = format!(
            "{}{}",
            String::from_utf8_lossy(&assessment.stdout),
            String::from_utf8_lossy(&assessment.stderr)
        );
        if assessment.exit_code != 0
            || assessment.timed_out
            || !assessment_text.contains("source=Notarized Developer ID")
        {
            return Err(backend_error(
                "Peekaboo app Gatekeeper/notary assessment failed",
            ));
        }
        return Ok(NotarizationAssessment::Passed);
    } else if require_cli_assessment {
        let assessment = run_tool(
            Path::new("codesign"),
            &["-vvvv", "-R=notarized", "--check-notarization", &path_text],
        )?;
        return classify_cli_notarization_assessment(
            asset.notarization.policy,
            assessment.exit_code,
            assessment.timed_out,
            assessment.signal,
        );
    }
    Ok(NotarizationAssessment::NotAssessed)
}

pub(super) fn classify_cli_notarization_assessment(
    policy: NotarizationPolicy,
    exit_code: i32,
    timed_out: bool,
    signal: Option<i32>,
) -> Result<NotarizationAssessment, CliError> {
    if timed_out {
        return Err(backend_error(
            "Peekaboo CLI notarization assessment timed out",
        ));
    }
    if signal.is_some() {
        return Err(backend_error(
            "Peekaboo CLI notarization assessment terminated by signal",
        ));
    }
    if exit_code == 0 {
        return Ok(NotarizationAssessment::Passed);
    }
    match policy {
        NotarizationPolicy::Required => {
            Err(backend_error("Peekaboo CLI notarization assessment failed"))
        }
        NotarizationPolicy::Waived => Ok(NotarizationAssessment::Waived),
    }
}
