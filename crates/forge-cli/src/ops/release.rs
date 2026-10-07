//! Repository-bound release creation and asset upload.
use super::github_write::{self as write, invalid};
use crate::backend::{BackendCall, BackendProgram};
use crate::cli::{GlobalFlags, ReleaseCommand};
use crate::error::ForgeError;
use nils_common::cli_contract::OutputFormat;
use serde::Serialize;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Serialize)]
struct ReleasePayload {
    provider: &'static str,
    repository: String,
    tag: String,
    url: Option<String>,
    assets: usize,
}
pub fn run(
    global: &GlobalFlags,
    command: ReleaseCommand,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let op = match &command {
        ReleaseCommand::Create(_) => "release.create",
        ReleaseCommand::Upload(_) => "release.upload",
    };
    let ctx = write::target(global, op)?;
    let mut argv = vec![OsString::from("release")];
    let mut notes_file = None;
    let (tag, asset_count) = match command {
        ReleaseCommand::Create(args) => {
            write::selector(&args.tag, op)?;
            if args.title.trim().is_empty() {
                return Err(invalid(
                    op,
                    "release_title_empty",
                    "release title must not be empty",
                ));
            }
            write::guard_text(&args.title, "release title")?;
            if let Some(target) = &args.target {
                write::selector(target, op)?;
            }
            let notes = write::text_file(&args.notes_file, op)?;
            write::guard_text(&notes, "release notes")?;
            let assets = asset_paths(&args.assets, op)?;
            let file = notes_file.insert(write::payload_file(notes.as_bytes(), op)?);
            argv.extend([
                "create".into(),
                args.tag.clone().into(),
                "--title".into(),
                args.title.into(),
                "--notes-file".into(),
                file.path().as_os_str().to_owned(),
            ]);
            if let Some(target) = args.target {
                argv.extend(["--target".into(), target.into()]);
            }
            if args.verify_tag {
                argv.push("--verify-tag".into());
            }
            if args.draft {
                argv.push("--draft".into());
            }
            if args.prerelease {
                argv.push("--prerelease".into());
            }
            if let Some(latest) = args.latest {
                argv.push(format!("--latest={latest}").into());
            }
            argv.extend(assets);
            (args.tag, args.assets.len())
        }
        ReleaseCommand::Upload(args) => {
            write::selector(&args.tag, op)?;
            let assets = asset_paths(&args.assets, op)?;
            argv.extend(["upload".into(), args.tag.clone().into()]);
            if args.clobber {
                argv.push("--clobber".into());
            }
            argv.extend(assets);
            (args.tag, args.assets.len())
        }
    };
    ctx.push_repo_override(&mut argv);
    let call = BackendCall::new(BackendProgram::Gh, argv);
    let result = write::execute(global, op, &ctx, call, format, |stdout| {
        Ok(ReleasePayload {
            provider: "github",
            repository: ctx.repo.clone().expect("target repository"),
            tag,
            url: super::issue_comment::first_url(&stdout),
            assets: asset_count,
        })
    });
    drop(notes_file);
    result
}
fn asset_paths(paths: &[PathBuf], op: &str) -> Result<Vec<OsString>, ForgeError> {
    paths
        .iter()
        .map(|path| {
            let path = path
                .canonicalize()
                .map_err(|_| invalid(op, "asset_unreadable", "release asset file is unreadable"))?;
            if !path.is_file() || std::fs::File::open(&path).is_err() {
                return Err(invalid(
                    op,
                    "asset_unreadable",
                    "release asset must be a readable file",
                ));
            }
            // Absolute paths cannot become backend options.
            Ok(path.into_os_string())
        })
        .collect()
}
