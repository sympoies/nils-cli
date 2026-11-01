## 2026-10-31 - agent-session: resume survives large history and broken session dirs
<!-- devlog-id: 2026-10-31-resume-scan-direct-lookup -->

### Result

- Deleted-source resume probes the one PROVIDER_SESSION_ID.jsonl filename per
  Claude project directory instead of scanning every transcript, so normal-size
  histories resolve at default scan settings.
- A managed session directory without a session.json is
  skipped with a stderr warning instead of refusing every resume.

### Why / context

- A real projects tree exceeded the default 250 ms slice and entry cap, and one
  directory without session.json refused all resumes.

### Evidence

- Regression tests: large-history resolve at defaults (nils-provider-resume)
  and broken-session-directory check-resume (agent-session integration).

### Links

- sympoies/nils-cli#3130
