## 2026-11-26 - Specify Console role attestation on session creation
<!-- devlog-id: 2026-11-26-issue-4077-console-attested-create -->

### Result

- Defined per-destination credential isolation, explicit relay grants and
  server-set steward attestation for fresh Console roots.
- Bound attested dispatch to relay capability discovery, a persisted
  operation receipt and a verified result.

### Why / context

- Review-handoff supervisors need broker-verified role provenance across
  session replacements. Console admission supplies that provenance without
  accepting forge identity from the request body.

### Evidence

- Cross-checked the binding against the merged resume-move transport and the
  existing create and broker identity paths.

### Links

- <https://github.com/sympoies/nils-cli/issues/4077>
