## 2026-10-31 - Correlate owned upload images across history rows
<!-- devlog-id: 2026-10-31-issue-3111-image-provenance -->

### Result

- History image descriptors and parts expose proven per-image user/tool origin
  and opaque same-session upload correlation while retaining row selectors.
  Claude Code 2.1.312 quoted owned upload strings project with ordered
  captions.

### Why / context

- User envelopes can mix user images and tool results. Only the existing
  managed attachment ownership and raster boundary can prove upload
  correlation; matching bytes, display names and working-directory read
  permission cannot. References hash owned storage identity without adding
  download authority or rewriting provider inputs.
- Explicit nonhuman Claude markers do not prove user attachment origin;
  nested tool-result provenance remains independently provable without
  changing carrier visibility or typed-parts suppression.

### Evidence

- Fresh synthetic upload/history/read regressions failed before production
  edits for missing Claude projection, origins and correlation. The focused
  history image route suite passes all 20 tests, including ownership refusals,
  captions, pagination, parts parity, independent uploads and response caps.
  Native provider/client acceptance remains separately scheduled.
- Review repair captured three fresh failures for explicit system,
  compact-summary and transcript-only image carriers before the metadata-only
  guard. The original positive provenance and Claude upload acceptance remain
  in the affected-package validation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3111>
- <https://github.com/sympoies/nils-cli/issues/3103>
