use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{commands, error::CliError, process, test_mode};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RunningApp {
    pid: i32,
    path: PathBuf,
}

// AppKit observes and quits exact application instances. The script never
// resets privacy grants or force-terminates a process. Arguments are data.
const LIST: &str = r#"
ObjC.import('AppKit');
function run(argv) {
    const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(argv[0]);
    const rows = [];
    for (let i = 0; i < apps.count; i++) {
        const app = apps.objectAtIndex(i);
        rows.push({pid: app.processIdentifier, path: ObjC.unwrap(app.bundleURL.URLByResolvingSymlinksInPath.path)});
    }
    return JSON.stringify(rows);
}
"#;

const QUIT: &str = r#"
ObjC.import('AppKit');
function run(argv) {
    const plan = JSON.parse(argv[1]);
    const apps = [];
    const running = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(argv[0]);
    for (const row of plan) {
        let app = null;
        for (let i = 0; i < running.count; i++) {
            const candidate = running.objectAtIndex(i);
            if (candidate.processIdentifier === row.pid) { app = candidate; break; }
        }
        if (app === null || app.isTerminated) continue;
        if (ObjC.unwrap(app.bundleIdentifier) !== argv[0] ||
            ObjC.unwrap(app.bundleURL.URLByResolvingSymlinksInPath.path) !== row.path) {
            throw new Error('running application identity changed; no quit authorized');
        }
        apps.push(app);
    }
    for (const app of apps) {
        if (!app.terminate) throw new Error('owned backend app refused to quit');
    }
    const deadline = Date.now() + 10000;
    while (apps.some(app => !app.isTerminated)) {
        if (Date.now() >= deadline) throw new Error('owned backend app did not quit');
        $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.05));
    }
    return '[]';
}
"#;

fn run_script(script: &str, bundle: &str, plan: Option<&str>) -> Result<Vec<RunningApp>, CliError> {
    let mut args = vec![
        "-l".into(),
        "JavaScript".into(),
        "-e".into(),
        script.into(),
        "--".into(),
        bundle.into(),
    ];
    if let Some(plan) = plan {
        args.push(plan.into());
    }
    let (env, removed) = commands::hardened_env(None);
    let output = process::run(
        Path::new("/usr/bin/osascript"),
        &args,
        &env,
        &removed,
        None,
        Duration::from_secs(15),
    )
    .map_err(|_| CliError::backend("unable to inspect or quit owned backend app instances"))?;
    if output.exit_code != 0 || output.timed_out || output.stdout_truncated {
        return Err(CliError::backend(
            "owned backend app reconciliation failed; quit backend instances manually and retry",
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| CliError::backend("backend app inspection returned an incompatible result"))
}

fn quit_plan(
    apps: &[RunningApp],
    owned: &[PathBuf],
    stable: &Path,
    retire_all: bool,
) -> Result<Vec<RunningApp>, CliError> {
    let mut pids = BTreeSet::new();
    let mut kept_stable = false;
    let mut quit = Vec::new();
    for app in apps {
        if app.pid <= 0 || !pids.insert(app.pid) || !owned.contains(&app.path) {
            return Err(CliError::backend(
                "an unowned Peekaboo instance is running; quit it manually before retrying",
            ));
        }
        if !retire_all && app.path == stable && !kept_stable {
            kept_stable = true;
        } else {
            quit.push(app.clone());
        }
    }
    Ok(quit)
}

pub(super) fn reconcile(
    owned: &[PathBuf],
    stable: &Path,
    bundle: &str,
    retire_all: bool,
) -> Result<(), CliError> {
    if test_mode::enabled() {
        test_mode::record_runtime_action(if retire_all {
            "retire-owned-apps"
        } else {
            "ensure-single-app"
        })
        .map_err(|_| CliError::backend("failed to record app lifecycle test action"))?;
        if !retire_all
            && std::env::var("NILS_MACOS_AGENT_TEST_PROBE_MODE").as_deref()
                == Ok("app_launch_failed")
        {
            return Err(CliError::backend("the stable backend app did not start"));
        }
        return Ok(());
    }
    let apps = run_script(LIST, bundle, None)?;
    let plan = quit_plan(&apps, owned, stable, retire_all)?;
    if !plan.is_empty() {
        let data = serde_json::to_string(&plan)
            .map_err(|_| CliError::backend("failed to encode app quit plan"))?;
        run_script(QUIT, bundle, Some(&data))?;
    }
    if retire_all {
        if !run_script(LIST, bundle, None)?.is_empty() {
            return Err(CliError::backend(
                "a backend app instance remained active; activation was stopped",
            ));
        }
        return Ok(());
    }
    let args = launch_args(stable);
    let (env, removed) = commands::hardened_env(None);
    let output = process::run(
        Path::new("/usr/bin/open"),
        &args,
        &env,
        &removed,
        None,
        Duration::from_secs(15),
    )
    .map_err(|_| CliError::backend("failed to launch the stable backend app"))?;
    if output.exit_code != 0 || output.timed_out {
        return Err(CliError::backend("the stable backend app did not start"));
    }
    let started = Instant::now();
    loop {
        let apps = run_script(LIST, bundle, None)?;
        if apps.len() == 1 && apps[0].path == stable {
            return Ok(());
        }
        quit_plan(&apps, owned, stable, false)?;
        if started.elapsed() >= Duration::from_secs(10) {
            return Err(CliError::backend(
                "expected exactly one stable backend app; quit duplicate instances and retry",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn launch_args(stable: &Path) -> Vec<String> {
    vec!["-g".into(), stable.to_string_lossy().into_owned()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn only_owned_instances_are_retired_and_one_stable_instance_is_kept() {
        let stable = PathBuf::from("/fixture/stable/Peekaboo.app");
        let old = PathBuf::from("/fixture/versions/v4.4.0/app/Peekaboo.app");
        let rows = vec![
            RunningApp {
                pid: 1,
                path: stable.clone(),
            },
            RunningApp {
                pid: 2,
                path: stable.clone(),
            },
            RunningApp {
                pid: 3,
                path: old.clone(),
            },
        ];
        let owned = vec![stable.clone(), old];
        assert_eq!(
            quit_plan(&rows, &owned, &stable, false)
                .unwrap()
                .iter()
                .map(|row| row.pid)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(quit_plan(&rows, &owned, &stable, true).unwrap().len(), 3);
        let foreign = vec![RunningApp {
            pid: 4,
            path: PathBuf::from("/fixture/other/Peekaboo.app"),
        }];
        assert!(quit_plan(&foreign, &owned, &stable, true).is_err());
        assert!(quit_plan(&[rows[0].clone(), rows[0].clone()], &owned, &stable, true).is_err());
    }

    #[test]
    fn launching_reuses_the_stable_application_instance() {
        assert_eq!(
            launch_args(Path::new("/fixture/stable/Peekaboo.app")),
            vec!["-g", "/fixture/stable/Peekaboo.app"]
        );
    }
}
