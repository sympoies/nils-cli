# agent-provider-gemini fixtures

This folder contains deterministic fixture inputs and outputs for `agent-provider-gemini` contract tests.

## Deterministic fixture policy

- Every fixture must be deterministic and reproducible in local and CI runs.
- Fixture IDs are immutable after publication; additive scenarios are allowed.
- Scenario timestamps, IDs, and limits must use fixed values rather than wall-clock data.

## Required fixture scenario coverage

The fixture set must include explicit coverage for:

- execute success mapping
- auth-state success mapping
- runtime unavailable fallback behavior
- unsupported capability behavior
- transport/network error mapping

## Redaction and secret hygiene

- Fixture payloads must be synthetic; do not include real API keys, bearer tokens, cookies, or user prompts.
- Redaction is required for all provider identifiers that could reveal private tenant data.
- Keep request/response envelopes contract-shaped while replacing sensitive values with stable placeholders.

## Fallback and unsupported behavior

- Missing runtime capabilities must map to deterministic fallback errors.
- Unsupported operations must map to explicit unsupported/unavailable categories and stable error codes.
- Tests must assert that fallback/unsupported paths do not silently degrade into success responses.
