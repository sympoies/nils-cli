# Webhook migration smoke procedure

Run this once, by an operator, before the first real use of `repo hook` or
`org hook` writes on production hooks, and again after any release that changes
how those commands write.

## Why

GitHub's documentation says that updating a webhook without supplying its
secret removes the secret (`PATCH /repos/{owner}/{repo}/hooks/{id}`). `hook
set-url` avoids that by using the configuration endpoint with only `url`, and
`hook set-active` sends only `active` with no `config` object. The second form
is the common idiom but is not stated in the documentation, and the tool never
sees the secret, so only a real delivery proves the secret survived. The
commands read each hook back and report `hook_readback_mismatch` if a
configured secret is no longer set; this procedure confirms that on a real
hook before it matters.

## Placeholders

`REPO` is `OWNER/NAME` of a throwaway repository you control, `HOOK_URL_A` and
`HOOK_URL_B` are two receivers you control that can verify the
`X-Hub-Signature-256` header, and `SECRET` is a throwaway secret held in the
environment variable `SMOKE_SECRET`. Use none of your real receivers, secrets,
or repositories.

## Procedure

1. Create a throwaway repository hook with a secret, either in the web UI
   (payload URL `HOOK_URL_A`, content type `application/json`, secret
   `SECRET`, push events) or, for an organization throwaway hook, with
   `forge-cli org hook create --org ORG --url HOOK_URL_A --events push
   --content-type json --secret-env SMOKE_SECRET`.
2. Push a commit (or use "Redeliver" in the hook's delivery list). Confirm a
   delivery reaches `HOOK_URL_A` and its signature verifies with `SECRET`.
3. Note the hook id with `forge-cli repo hook list --repo REPO` (or `org hook
   list --org ORG`). Confirm it shows `HOOK_URL_A`, `active: true`, and
   `secret: "[redacted]"`. If no secret marker is shown, stop: the hook has no
   secret recorded, so the rest of the procedure proves nothing.
4. Switch the URL: `forge-cli repo hook set-url --repo REPO --hook ID --from
   HOOK_URL_A --to HOOK_URL_B`. Expect `status: updated`.
5. Confirm `hook list` shows `HOOK_URL_B` and the secret still marked
   `[redacted]`; trigger a delivery and confirm it reaches `HOOK_URL_B` and its
   signature still verifies with the original `SECRET`.
   Recover failures during the switch: record the stop window as RFC 3339
   `SINCE` (inclusive) and `UNTIL` (exclusive), then preview with
   `forge-cli repo hook redeliver --repo REPO --hook ID --status failed
   --since SINCE --until UNTIL --dry-run`. Remove `--dry-run` to attempt each
   selected id once.

   For organization hooks use `org hook redeliver --org ORG`
   with the same selection flags. For multiple hooks, put `action: "redeliver"`
   rows in the existing `hook batch --file FILE` (see the
   [row format](../specs/forge-webhooks-v1.md#hook-redeliver)). GitHub only
   retains deliveries eligible for redelivery for the
   [past three days](https://docs.github.com/en/webhooks/testing-and-troubleshooting-webhooks/redelivering-webhooks),
   so run this promptly. Check every row's observed delivery status and the
   receiver; `pending` means the asynchronous attempt is not yet confirmed.
   Resolve failed/unknown rows before retrying to avoid duplicate effects.
6. Switch back with the same command with `--from` and `--to` swapped. Expect
   `status: updated`; confirm `hook list` shows `HOOK_URL_A` and a delivery to
   `HOOK_URL_A` still verifies with `SECRET`.
7. Deactivate: `forge-cli repo hook set-active --repo REPO --hook ID
   --inactive`. Expect `status: updated`; `hook list` shows `active: false`
   and the secret still marked `[redacted]`; a push produces no delivery.
8. Reactivate: `forge-cli repo hook set-active --repo REPO --hook ID
   --active`. Expect `status: updated`; `hook list` shows `active: true`;
   trigger a delivery and confirm its signature still verifies with the
   original `SECRET`.
9. Run steps 4 to 8 once more with `--dry-run` first and confirm each reports
   `would_update` and changes nothing.
10. Delete the throwaway hook by hand (web UI). The commands never delete.

## Pass and stop conditions

Every change reports `updated` and every delivery after a change verifies with
the original secret: the writes are safe, proceed with the migration.

If any command reports `hook_readback_mismatch`, or a delivery stops verifying,
stop. A secret was dropped or changed: do not run the migration or any batch,
delete the throwaway hook, and report the exact command, its output, and the
GitHub documentation sentence you relied on to the maintainers.
