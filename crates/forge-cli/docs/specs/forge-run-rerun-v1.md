# Workflow run rerun contract v1

`forge-cli run rerun <run-id> [--job <job-id> | --failed-only]
--expected-head <full-sha>` requests one GitHub Actions rerun without creating a
commit or requiring a workflow dispatch trigger. The default reruns the whole
run, including a completed attempt whose conclusion was cancelled.

## Target and authority

The repository and authority come from the ordinary global flags or selected
remote. Run and job IDs must be positive integers. The expected head must be a
full 40- or 64-digit hexadecimal SHA. The run must be completed, its ID and head
must match the request, and a selected job must belong to that run, head, and
latest attempt. `--job` and `--failed-only` are mutually exclusive.

Before writing, the CLI re-reads the run and refuses if the attempt changed.
Every associated PR is fetched again and its current head must equal the run
head. Push-triggered runs without embedded PR associations discover associated
PRs through the commit API. A PR-triggered run with missing associations fails
closed with `run_pr_head_unavailable`. Fork PR metadata can therefore require
manual investigation. No stale-head override is provided.

This is a read-before-write guard: GitHub's rerun endpoint has no atomic head
compare-and-swap. A PR update or another rerun after the last preflight read
cannot be excluded. The command never infers permission to rerun a stale head.

All reads and the write use the normal identity-scoped backend runner. With an
identity policy, `api_write` authorization, selected credentials, actor
verification, output redaction, and the existing identity audit apply to the
entire invocation. Without a policy, existing backend authentication applies.
The receipt records the authenticated actor. `operation-effect` declares
`run.rerun`, `mutation`, and `network_write`.

## One write and its receipt

The command POSTs exactly once to the selected
[GitHub rerun endpoint](https://docs.github.com/en/rest/actions/workflow-runs).
A job rerun also reruns its dependent jobs; failed-only uses
`rerun-failed-jobs`. REST writes are never replayed by the GraphQL retry gate.
An error or timeout during POST is ambiguous: inspect the provider before
considering a separately authorized invocation.

After acceptance, bounded reads inspect the new attempt and paginate its jobs.
`--receipt-timeout-secs` defaults to 10 and accepts 0 through 60. Zero performs
one probe with a 15-second backend budget; a positive value bounds receipt
reads and waiting by that many seconds. Preflight and the POST each have their
own 15-second backend timeout.

JSON success uses `cli.forge-cli.run.rerun.v1` with `provider`, `repository`,
`run_id`, `head_sha`, `actor`, `selection` (`all`, `failed`, or `job`),
`selected_job`, `previous_attempt`, `rerun_requested: true`, `url`,
`receipt_status`, `receipt_error`, `attempt`, `attempt_url`, and `jobs`.
Each job has its new `id`, `name`, and `html_url`.

- `observed`: the next attempt and nonempty job list were read from GitHub;
  `attempt` and `attempt_url` identify that attempt.
- `pending`: the accepted write has not produced a complete readable receipt
  within the budget. Attempt fields are null and jobs are empty. A read failure
  supplies its stable error kind in `receipt_error`.
- `superseded`: a later concurrent attempt prevents attribution of the next
  attempt. Attempt fields are null and jobs are empty.

Receipt read failures retain successful write acceptance and never trigger a
second POST. A pending or superseded receipt is not evidence that the rerun
failed. Text output includes actor, status, attempt URL, and job IDs when
observed. `--dry-run` performs the same read-only preflight and emits the POST
plan without a mutation or receipt polling.

## Provider scope

GitHub is implemented. GitLab and Local return `provider_unsupported`;
identity enforcement may refuse GitLab earlier with
`identity_provider_unsupported`. GitLab pipeline retry and its shared identity
foundation are explicitly deferred. The current identity API adapter rejects
non-GitHub backends, and its shared credential/actor verification is
GitHub-specific. GitLab's
[pipeline retry API](https://docs.gitlab.com/api/pipelines/#retry-jobs-in-a-pipeline)
retries failed or cancelled jobs in place and has no GitHub-style run attempt
number. This command does not claim GitLab parity or live-provider qualification.
