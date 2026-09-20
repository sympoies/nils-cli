use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOutput};
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::sync::OnceLock;

fn completion_bin() -> PathBuf {
    bin::resolve("claude-cli")
}

fn run_uncontained(args: &[&str]) -> CmdOutput {
    let bin = completion_bin();
    cmd::run(&bin, args, &[], None)
}

fn completion_zsh() -> &'static str {
    static OUTPUT: OnceLock<String> = OnceLock::new();
    OUTPUT
        .get_or_init(|| {
            let output = run_uncontained(&["completion", "zsh"]);
            assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
            output.stdout_text()
        })
        .as_str()
}

fn assert_contains_all(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "missing completion contract token: {needle}"
        );
    }
}

#[test]
fn completion_contract_includes_top_level_command_families() {
    let script = completion_zsh();
    assert_contains_all(
        script,
        &[
            "'agent:Agent command group' \\",
            "'auth:Authentication command group' \\",
            "'config:Configuration command group' \\",
            "'prompt-segment:Prompt-segment command group' \\",
            "'usage:Read Claude usage from OAuth, Claude CLI, or cache' \\",
            "'completion:Export shell completion script' \\",
        ],
    );
}

#[test]
fn completion_contract_is_context_aware_across_command_families() {
    let script = completion_zsh();
    assert_contains_all(
        script,
        &[
            "curcontext=\"${curcontext%:*:*}:claude-cli-command-$line[1]:\"",
            "curcontext=\"${curcontext%:*:*}:claude-cli-agent-command-$line[1]:\"",
            "curcontext=\"${curcontext%:*:*}:claude-cli-auth-command-$line[1]:\"",
            "curcontext=\"${curcontext%:*:*}:claude-cli-config-command-$line[1]:\"",
            "curcontext=\"${curcontext%:*:*}:claude-cli-prompt-segment-command-$line[1]:\"",
        ],
    );
}

#[test]
fn completion_contract_declares_enum_value_candidates() {
    let script = completion_zsh();
    assert_contains_all(
        script,
        &[
            "'--source=[Usage source to read]:SOURCE:(auto oauth cli cache)' \\",
            "'--runtime=[Runtime profile (default\\: safe)]:mode:(safe inherited)' \\",
            "'--effort=[Claude effort override]:level:(low medium high xhigh max)' \\",
            "':shell -- Shell to generate completion script for:(bash zsh)' \\",
        ],
    );
}

#[test]
fn completion_contract_format_values_include_text_and_json() {
    let script = completion_zsh();
    let format_candidates = script.match_indices(":format:(text json)'").count();
    assert!(
        format_candidates >= 3,
        "expected at least 3 format candidate entries, got {format_candidates}"
    );
    assert_contains_all(
        script,
        &[
            "'--format=[Output format (\\`text\\` or \\`json\\`)]:format:(text json)' \\",
            "'(--format)--json[Hidden alias for \\`--format json\\`]' \\",
        ],
    );
}

#[test]
fn completion_contract_covers_the_usage_cache_and_debug_flags() {
    let script = completion_zsh();
    assert_contains_all(
        script,
        &[
            "'-c[Clear the usage cache file before querying]' \\",
            "'--clear-cache[Clear the usage cache file before querying]' \\",
            "'-d[Report bounded source-attempt diagnostics on stderr]' \\",
            "'--debug[Report bounded source-attempt diagnostics on stderr]' \\",
        ],
    );
}
