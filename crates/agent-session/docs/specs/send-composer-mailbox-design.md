# Send, composer and mailbox-wake design

Status: proposed design; documentation only. This is step 1 of
[#3186](https://github.com/sympoies/nils-cli/issues/3186). It neither changes
admission nor authorizes implementation, a rollout, issue closure or live probes.
Ownership: crate-local design record in the existing specs location.

## Evidence boundary and conclusion

Audit date: 2026-11-13. Source is pinned to main
[`35c22691cfa445b79ee7f14eb97a745a26a9f812`](https://github.com/sympoies/nils-cli/tree/35c22691cfa445b79ee7f14eb97a745a26a9f812).
The history window is 2026-10-02 through that pin, approximately six weeks.
The matrix groups merged changes with the same decision; a grouped cell does
not assert that every PR changed every part of that decision.

Method: inspect `git log --since=2026-10-02 --name-only` for `agent-session`
and `agent-hook`, then the relevant commit bodies, affected owners and tests.
Include fixes to send admission, composer reading, paste/submit proof, notices,
notification generation and wake dispatch, plus the features that established
those boundaries. Exclude unrelated changes even when a large shared file was
touched. Read each named issue with `forge-cli issue view --with-comments`;
issue state is a point-in-time provider observation, not proof of acceptance.
No Rust tests, model-provider calls or real sessions were run for this audit.

**Conclusion (inference): there is no single root cause.** Render-derived
composer/ownership decisions recur across sends and terminal wakes (R1).
Conflating dispatch, acknowledgment and positive confirmation recurs across
terminal, native and mailbox attempts (R2). Both justify shared internal
contracts with multiple existing consumers. Notification scheduling (R3),
machine-origin history (R4), provider integration (R5) and caller command intent
(R6) have distinct evidence and acceptance. They must not be folded into a
single screen-parser rewrite. No frequency percentage is inferred from grouped
rows or from the earlier, shorter audit.

The [original matrix](https://github.com/sympoies/nils-cli/issues/3186#issuecomment-6560861686)
and [design comment](https://github.com/sympoies/nils-cli/issues/3186#issuecomment-6560958124)
used an earlier main. Since then client-id reconciliation, the pane corpus,
scheduler notices and additional attestation have merged. This proposal reuses
those owners. The later
[idle-Claude wake report](https://github.com/sympoies/nils-cli/issues/3186#issuecomment-6566011074)
establishes a reported delivery problem, not a proven root cause or a passing
reproduction. A guarded Enter doing nothing does not prove an empty composer,
provider readiness, an offered native call or consumption of a generation.

## Merged-fix matrix

Inputs: **screen** means rendered pane/cursor/style data; **structured** means
provider protocol/hooks or an authenticated native request; **local** means
persisted identity, claims, generations or configuration; **recorded text**
means provider transcript content. Exact bytes can validate a positively bound
operation but text similarity alone does not grant origin or delivery authority.

### Composer, paste and terminal confirmation

| Row / cause | Merged PRs | Decision and input | Failure | Delivered correction / remaining limit |
| --- | --- | --- | --- | --- |
| A01 / R1 | [#2451](https://github.com/sympoies/nils-cli/pull/2451), [#2523](https://github.com/sympoies/nils-cli/pull/2523), [#2535](https://github.com/sympoies/nils-cli/pull/2535), [#2582](https://github.com/sympoies/nils-cli/pull/2582), [#2924](https://github.com/sympoies/nils-cli/pull/2924) | Empty or owner draft? screen SGR, OSC-8, separators and padding | Hints, hyperlinks and layout blanks read as drafts or unreadable input. | Recognize bounded styling/layout forms while retaining real-draft refusal; #2924 covers the three-space Codex form. |
| A02 / R1 | [#2587](https://github.com/sympoies/nils-cli/pull/2587), [#3231](https://github.com/sympoies/nils-cli/pull/3231), [#3318](https://github.com/sympoies/nils-cli/pull/3318) | Where is the composer? screen borders, row position and choice tokens | Title-only borders, clipped first rows or version-like suggestions resemble unsupported layouts or choices. | Constrain box/first-row and numbered-choice parsing; no arbitrary border or suggestion-text authority. |
| A03 / R1 | [#2622](https://github.com/sympoies/nils-cli/pull/2622), [#2432](https://github.com/sympoies/nils-cli/pull/2432) | May direct reply or terminal wake start? screen composer plus identity/activity | Native dispatch or wake can ignore an unsent human draft. | Require observable empty admission, preserve drafts and allow later retry after safe clearance. |
| A04 / R1 | [#2592](https://github.com/sympoies/nils-cli/pull/2592) | Is exact terminal input representable before paste? caller bytes and screen capability | A pasted internal newline makes verification impossible after input has already changed. | Refuse unsupported multiline guarded text before paste; ordinary-send trailing-line-break handling is separate from exact notice bytes. |
| A05 / R1 | [#2805](https://github.com/sympoies/nils-cli/pull/2805), [#3028](https://github.com/sympoies/nils-cli/pull/3028), [#3038](https://github.com/sympoies/nils-cli/pull/3038), [#3110](https://github.com/sympoies/nils-cli/pull/3110) | Does the whole wrapped input belong to this send? screen plus cursor round trips | Omitted wrap spaces, wide cells and hidden trailing blanks make visible-prefix matching unsafe. | Prove logical start, character/cell length and actual end; reject hidden owner suffix and ambiguous navigation. |
| A06 / R1,R2 | [#2995](https://github.com/sympoies/nils-cli/pull/2995) | Retry observation or end proof? screen and original deadline | Transient redraw looks unreadable; a readable mismatch could later disappear. | Retry only transient observations within the existing budget; identity checks persist and a readable conflict is terminal. |
| A07 / R1,R2 | [#2598](https://github.com/sympoies/nils-cli/pull/2598), [#2606](https://github.com/sympoies/nils-cli/pull/2606) | Can cleanup remove only owned text? earlier exact observation and fresh screen/cursor proof | Refused wrapped paste remains, or prefix deletion could erase appended human input. | Bounded exact/prefix ownership proof and post-key verification; ambiguous or mixed drafts stay preserved. |
| A08 / R1 | [#3327](https://github.com/sympoies/nils-cli/pull/3327) | Is an opaque Claude paste owned by this attempt? empty admission, screen token and navigation | A collapsed block cannot be treated like readable text or an admission-time owned draft. | Bind a lone token to this send and prove atomic navigation before Enter/Backspace; keep unknown and owner-edit controls. |
| A09 / R1,R2 | [#3346](https://github.com/sympoies/nils-cli/pull/3346), [#3348](https://github.com/sympoies/nils-cli/pull/3348), [#3471](https://github.com/sympoies/nils-cli/pull/3471) | Can ownership/identity proof finish? screen subprocesses and deadline | Round trips exhaust short budgets; timeout can be misreported as missing identity. | Bound proof/cleanup latency by the documented owner budget and name expiry; never infer ownership from time elapsed. |
| A10 / R1 | [#3360](https://github.com/sympoies/nils-cli/pull/3360), [#3453](https://github.com/sympoies/nils-cli/pull/3453), [#3461](https://github.com/sympoies/nils-cli/pull/3461) | Is a Codex envelope exact/owned? screen physical rows, header wrap, Unicode count and atomic token | Blank rows, wrapped headers and collapsed notices defeat naive line matching. | Keep physical coordinates and logical boundary/count proofs, including owned-token cleanup; not a provider-independent paste proof. |
| A11 / R1,R2 | [#2070](https://github.com/sympoies/nils-cli/pull/2070), [#2589](https://github.com/sympoies/nils-cli/pull/2589), [#2910](https://github.com/sympoies/nils-cli/pull/2910) | Was a Claude handoff queued? screen queue evidence and pre-submit baseline | No turn starts immediately; stale/sibling queue text can falsely confirm a new request. | Observe the actual queued input during the wait; require new full-text evidence under the delivery lock. |
| A12 / R2 | [#2621](https://github.com/sympoies/nils-cli/pull/2621), [#2654](https://github.com/sympoies/nils-cli/pull/2654), [#2663](https://github.com/sympoies/nils-cli/pull/2663), [#2724](https://github.com/sympoies/nils-cli/pull/2724), [#2800](https://github.com/sympoies/nils-cli/pull/2800) | Did Enter submit, and may it repeat? exact screen input and verified runtime exit | Swallowed Enter or late rendering causes false success, stuck wake text or duplicate startup paste. | Retry only Enter with fresh exact proof; never repaste; verified exit can confirm; unproved/empty aftermath stays unknown and cleanup needs ownership. |
| A13 / R1 diagnostics | [#3207](https://github.com/sympoies/nils-cli/pull/3207), [#3302](https://github.com/sympoies/nils-cli/pull/3302), [#3465](https://github.com/sympoies/nils-cli/pull/3465) | Why is input unreadable? bounded screen-reader result | One generic composer refusal hides capture/layout/dialog/queued-hint distinctions. | Expose fixed observation reasons, including multi-row drafts, without pane text or weaker admission. |
| A14 / R1 contract | [#3216](https://github.com/sympoies/nils-cli/pull/3216) | Does the frozen reader keep its verdict? sanitized/derived screen corpus | A new render fix can silently widen unrelated admission. | Pin exact and direct reader outcomes with escape bytes intact in the normal unit suite; not qualification of unseen provider builds. |
| A15 / R1 policy | [#3255](https://github.com/sympoies/nils-cli/pull/3255), [#3457](https://github.com/sympoies/nils-cli/pull/3457), [#3486](https://github.com/sympoies/nils-cli/pull/3486) | May a stale draft be saved/cleared or discarded? exact observation, age and local ownership | Stale drafts block wakes; runtime residue can be saved as human input; saved-text cleanup can select the notice proof incorrectly. | Persist byte-exact human drafts before owned clearing; distinguish incarnation-bound notice residue; persist replacement projection before discard; use the correct cleanup proof. |
| A16 / test harness | [#3305](https://github.com/sympoies/nils-cli/pull/3305), [#3516](https://github.com/sympoies/nils-cli/pull/3516), [#3328](https://github.com/sympoies/nils-cli/pull/3328) | When may a fixture dispatch? test budget guard and receipt retry deadline | Tests expire early or bypass backoff; independent cases cost unnecessary serialized time. | Hold the shared budget guard, wait for real retry eligibility and batch independent cases; no production admission change. |

### Native submission and notice provenance

| Row / cause | Merged PRs | Decision and input | Failure | Delivered correction / remaining limit |
| --- | --- | --- | --- | --- |
| B00 / R2 foundations | [#2394](https://github.com/sympoies/nils-cli/pull/2394), [#2540](https://github.com/sympoies/nils-cli/pull/2540), [#2980](https://github.com/sympoies/nils-cli/pull/2980) | Which request owns the attempt and history row? durable key, original payload/identity and structured turn proof | Retried calls and delayed provider rows can become duplicate or unassociated input. | Establish guarded once-only claims, durable prompt receipts/idle admission and bound history association; later reconciliation must preserve those owners. |
| B01 / R2,R5 | [#2158](https://github.com/sympoies/nils-cli/pull/2158), [#2460](https://github.com/sympoies/nils-cli/pull/2460), [#2468](https://github.com/sympoies/nils-cli/pull/2468), [#2503](https://github.com/sympoies/nils-cli/pull/2503) | Can native Codex submit and confirm? structured thread state, identity, account gate and composer screen | Unsupported-profile rejection, startup lock inversion or a lock held across synchronous hooks prevents progress. | Validate supported binding, use native start/compact, release record lock after ack for hooks, keep account/key fences and revalidate identity before return. |
| B02 / R2 | [#2680](https://github.com/sympoies/nils-cli/pull/2680), [#2718](https://github.com/sympoies/nils-cli/pull/2718), [#3205](https://github.com/sympoies/nils-cli/pull/3205) | Can an existing request become confirmed? write-stage evidence, ack/turn or client id plus exact recorded input | Lost ack, delayed history or runtime exit leaves ambiguous receipts. | Reconcile only under original identity/account/runtime fences; client id plus unique exact input can recover lost ack without dispatch; missing/duplicate/mismatched proof remains unknown. |
| B03 / R2,R5 | [#3221](https://github.com/sympoies/nils-cli/pull/3221), [#3508](https://github.com/sympoies/nils-cli/pull/3508) | Is provider idle verifiable? structured status/hooks, pending ingress and attention | Loaded Codex systemError is mistaken for running; missing completion resembles all other unknown idle states. | Admit the supported systemError state; retain active/unknown/identity refusal; name missing completion. Does not prove the cause of #3215. |
| B04 / R2 recorded text | [#2376](https://github.com/sympoies/nils-cli/pull/2376), [#2391](https://github.com/sympoies/nils-cli/pull/2391), [#2743](https://github.com/sympoies/nils-cli/pull/2743), [#2784](https://github.com/sympoies/nils-cli/pull/2784), [#2796](https://github.com/sympoies/nils-cli/pull/2796) | Where is accepted input observed? bound transcript source, queued row shape and staged attachment label | Truncated discovery, configured root links, list-form queue rows or rewritten image labels hide real input evidence. | Resolve the bound source with ownership checks; parse actual queue shape and newly appended attachment/label proof. No image-content or arbitrary-text match authority. |
| B05 / R2 diagnostics | [#2477](https://github.com/sympoies/nils-cli/pull/2477) | Which native step stalled? opt-in classified RPC timing/equality | An outward unknown gives no bounded stage evidence. | Private bounded diagnostic trace; no provider payload, retry, admission change or inferred success. |
| B06 / R5 containment | [#2997](https://github.com/sympoies/nils-cli/pull/2997), [#3090](https://github.com/sympoies/nils-cli/pull/3090) | May Claude terminal carry an exact notice? input bytes and available proof | Multiline collapse/trailing spaces cannot supply exact pre-Enter envelope proof. | Broad preclaim/prepaste terminal-notice refusal replaces the narrower whitespace limitation; queueing alone does not provide the missing proof. |
| B07 / R2,R4,R5 | [#2988](https://github.com/sympoies/nils-cli/pull/2988), [#3104](https://github.com/sympoies/nils-cli/pull/3104) | May a native Claude notice count as delivered/machine-origin? canonical bytes, authenticated module request and exact new provider row | A pasted marker or native return alone does not prove exact input and history origin. | Reuse codec and durable one-shot authorization; require exact native result and row proof, then project notice bytes/role; no terminal replay on unknown. |
| B08 / R5 | [#3118](https://github.com/sympoies/nils-cli/pull/3118), [#3140](https://github.com/sympoies/nils-cli/pull/3140), [#3320](https://github.com/sympoies/nils-cli/pull/3320) | Can the Claude module attach/load? tested external version, loader API and launch metadata | Handle storage breaks loading; exact build gate or untested drift obscures supported coverage. | Correct hook call shape; reviewed tested range and pinned bootstrap/version checks. Range widening still needs its own evidence. |
| B09 / R5 diagnostics | [#3137](https://github.com/sympoies/nils-cli/pull/3137), [#3141](https://github.com/sympoies/nils-cli/pull/3141), [#3335](https://github.com/sympoies/nils-cli/pull/3335) | Why did native admission refuse? bound transcript baseline/readiness result | Zero-turn clean absence is confused with unsafe/ambiguous lookup or other attachment failure. | Separate baseline-not-found from unavailable/unprovable and expose fixed preclaim reasons; postclaim uncertainty never becomes safe non-attempt. |
| B10 / test isolation | [#3147](https://github.com/sympoies/nils-cli/pull/3147) | Does provider contract validation use isolated state? test-owned home/environment | An installed provider check may touch contributor configuration. | Run the contract check in a private fixture home with cleared environment; preserve explicit unsupported/absent-provider skip. |
| B11 / R2,R4 storage | [#3339](https://github.com/sympoies/nils-cli/pull/3339), [#3351](https://github.com/sympoies/nils-cli/pull/3351) | Can notice claims retire without losing dedupe/provenance? immutable claims, authenticated archive pages and reservations | Live capacity saturates or history reads scan unbounded proof; retirement can lose permanent key fences. | Bound live segments and indexed proof lookup, seal immutable history and preserve reservations/durable publication; no new evidence store proposed here. |
| B12 / R2,R4 | [#3400](https://github.com/sympoies/nils-cli/pull/3400), [#3452](https://github.com/sympoies/nils-cli/pull/3452) | How do schedulers, operator notices and mailbox start/steer carry canonical provenance? structured client id, exact physical rows and existing notice claim | Raw scheduler input appears human; some native busy/mailbox calls lack correlation or release-bound identity fences. | Reuse notice owner for scheduler and controller/operator attestation, bind actual start/steer write, exact finite row proof and consumed unknowns; preserve scheduler non-idle cause/turn/deadline fences. |
| B13 / R4 | [#3267](https://github.com/sympoies/nils-cli/pull/3267), [#3487](https://github.com/sympoies/nils-cli/pull/3487), [#3491](https://github.com/sympoies/nils-cli/pull/3491), [#3498](https://github.com/sympoies/nils-cli/pull/3498), [#3500](https://github.com/sympoies/nils-cli/pull/3500) | Is a row human input in every history/preview reader? recorded row plus shared classification/receipt cutoff | Notices leak into human previews or readers disagree about classification. | Share classification/binding across forward/reverse, exact/incremental and live preview reads; keep provider admission evidence separate from human selection. |
| B14 / R4 | [#3485](https://github.com/sympoies/nils-cli/pull/3485), [#3494](https://github.com/sympoies/nils-cli/pull/3494), [#3507](https://github.com/sympoies/nils-cli/pull/3507) | Does launch text have machine authority? launch-owned receipt and bounded physical row | Metadata dropped across launch relay or a pasted header mislabels a human row. | Carry launch metadata to the receipt owner; bind exact launch row and session cutoff; later unclaimed headers remain human. |
| B15 / R4 | [#3579](https://github.com/sympoies/nils-cli/pull/3579), [#3595](https://github.com/sympoies/nils-cli/pull/3595), [#3609](https://github.com/sympoies/nils-cli/pull/3609) | Is a last-prompt/queue echo the bound launch notice? receipt plus bounded recorded-text comparison | Full, truncated or flattened echoes repopulate human prompt previews. | Mask only receipt-bound echoes with anchored truncation/flattening checks; keep unbound human headers visible. This projection evidence never authorizes send or replay. |

### Wake scheduling, routing and distinct neighboring causes

| Row / cause | Merged PRs | Decision and input | Failure | Delivered correction / remaining limit |
| --- | --- | --- | --- | --- |
| C01 / R1,R3 | [#2560](https://github.com/sympoies/nils-cli/pull/2560), [#2877](https://github.com/sympoies/nils-cli/pull/2877), [#3199](https://github.com/sympoies/nils-cli/pull/3199), [#3238](https://github.com/sympoies/nils-cli/pull/3238), [#3345](https://github.com/sympoies/nils-cli/pull/3345) | What wake text fits the selected route? screen width versus native/hook delivery plus local format | Compact letters are unreadable; long fixed text wraps; old literal formats outlive route changes. | Shorten the historical typed wake, then migrate current producers/recognizers to readable notice text and retire old formats; retained unknown attempts do not become replayable. |
| C02 / R2,R3 | [#2299](https://github.com/sympoies/nils-cli/pull/2299), [#2398](https://github.com/sympoies/nils-cli/pull/2398), [#2430](https://github.com/sympoies/nils-cli/pull/2430), [#2934](https://github.com/sympoies/nils-cli/pull/2934) | Which generation is pending/resolved? local receipt, canonical unread content and queue provenance | An older unknown stalls newer mail; drained mail still wakes; inspection mutates state or loses diagnostic history. | Keep active attempt fences, recover newer queue provenance, resolve drained generations and retain bounded content-free outcomes; inspection is observational. |
| C03 / R3 policy | [#2494](https://github.com/sympoies/nils-cli/pull/2494), [#3017](https://github.com/sympoies/nils-cli/pull/3017) | When should routine versus urgent mail wake? recipient-owned policy, categories, time and persisted generation | Routine bursts interrupt repeatedly; sender-controlled policy or restart can bypass coalescing. | Destination-owned routine window; urgent/service bypass; persist the window and consumed unknown attempts, with matching current writers/readers. |
| C04 / R2,R3 | [#3166](https://github.com/sympoies/nils-cli/pull/3166), [#3234](https://github.com/sympoies/nils-cli/pull/3234), [#3430](https://github.com/sympoies/nils-cli/pull/3430), [#3490](https://github.com/sympoies/nils-cli/pull/3490) | When is the generation consumed? local CAS and authenticated native release | Hook/controller compete or an unoffered native timeout consumes a wake permanently. | Use canonical notice route; writer-disabled Codex hook leaves controller work queued; consume Claude generation at authenticated release, retain preoffer retries and postclaim unknown fences; avoid repeated unchanged deferral writes. |
| C05 / R3 upstream state | [#3269](https://github.com/sympoies/nils-cli/pull/3269), [#3353](https://github.com/sympoies/nils-cli/pull/3353), [#3618](https://github.com/sympoies/nils-cli/pull/3618) | May startup/resume reach idle wake checks? structured primary-thread idle and retained startup/activity | Retained-turn or prompt-free startup remains starting; unbound terminal-started settlement needs distinct proof. | Settle verified startup before input projection; retain active/loading/pending-ingress controls; add unbound terminal-started wake-settlement coverage in #3618. Startup is not composer truth. |
| C06 / R3 | [#3443](https://github.com/sympoies/nils-cli/pull/3443) | Does a claim mean this recipient is busy? local claim/operation/quiescence and route kind | An idle recipient holding only its own advisory claim is never woken. | Idle wake ignores claim alone but blocks active/uncertain operations; in-turn steer retains the claim fence and records the blocking condition. |
| C07 / R3 diagnostics | [#3441](https://github.com/sympoies/nils-cli/pull/3441) | Why is a queued wake deferred? fixed route/admission detail | provider-not-ready hides disabled writer, unavailable control or actual quiescence refusal. | Record bounded reason_detail/condition without payload; keep repeated deferrals observable and typed. |
| C08 / R3,R5 | [#3527](https://github.com/sympoies/nils-cli/pull/3527) | Which Claude wake route is supported before input? known launch/config absence versus uncertain attached receiver | Module never attached makes a notice-only wake permanently unavailable. | Select guarded one-line text only for known configuration absence; attached-but-unready stays queued on native route. Record cause; never switch after an attempted/unknown notice. |
| C09 / R3 wording | [#3254](https://github.com/sympoies/nils-cli/pull/3254), [#3437](https://github.com/sympoies/nils-cli/pull/3437), [#3535](https://github.com/sympoies/nils-cli/pull/3535), [#3545](https://github.com/sympoies/nils-cli/pull/3545) | What action does a wake request? trusted executable/recipient plus canonical guidance | A generic wake has no usable read action or names the wrong executable/recipient. | Share guidance, include validated recipient-bound inbox/read commands, and use one line on the text route; absent trusted helper defers rather than using PATH. |
| C10 / R3 metadata | [#3315](https://github.com/sympoies/nils-cli/pull/3315), [#3252](https://github.com/sympoies/nils-cli/pull/3252), [#3432](https://github.com/sympoies/nils-cli/pull/3432), [#2442](https://github.com/sympoies/nils-cli/pull/2442) | What urgency/recovery context is available? local lineage, reply deadline and quota state | Sender relationship or quota pressure is unclear; repeated sends cannot repair a full mailbox. | Expose bounded lineage/deadline metadata and quota/read-ACK guidance without private bodies. Not an input-state parser fix. |
| D01 / R6,R1 UI | [#3064](https://github.com/sympoies/nils-cli/pull/3064), [#3113](https://github.com/sympoies/nils-cli/pull/3113), [#3115](https://github.com/sympoies/nils-cli/pull/3115), [#3131](https://github.com/sympoies/nils-cli/pull/3131) | Is owner input a command, and did its UI open? caller token/origin and screen picker | Paths look like commands; valid model/effort picker or wrapped footer is not observed. | Keep first-token/path rule and authenticated command policy; distinguish command_dispatched from model submission with constrained UI evidence. Explicit intent is separately designed in #3389. |
| D02 / R5 transport | [#2057](https://github.com/sympoies/nils-cli/pull/2057), [#2066](https://github.com/sympoies/nils-cli/pull/2066) | Which terminal bytes/renderer does the external tool use? tmux version behavior and launch configuration | Raw escape bytes are sanitized or renderer selection drifts. | Use the external tmux 3.7 raw-paste contract with its explicit older-tmux handling; pin managed main-screen rendering. Not a repository old-format allowance. |
| D03 / R5 boundary | [#2911](https://github.com/sympoies/nils-cli/pull/2911) | May input use the current binding? structured applied account/thread binding | Failed binding allows text or printable keys to reach the wrong runtime context. | Fence literal/printable input and broker-validated repair membership; account switching itself stays out of this design. |
| D04 / adjacent, distinct | [#2802](https://github.com/sympoies/nils-cli/pull/2802), [#3558](https://github.com/sympoies/nils-cli/pull/3558) | Can a mailbox send body or queued remote receipt be read? regular-file source / committed registry snapshot and journal | Source IO is mislabeled as content failure; registry contention prevents answering already queued remote sends. | Preserve safe source handling and classify IO; authenticated snapshot lookup answers remote receipt/replay with typed recovery. Transport/storage availability is not composer or wake proof. |

## Named-issue matrix and acceptance ownership

Verified with bodies **and comments** on 2026-11-13: only #2691 is open among
the ten seed issues. Closed does not mean all original live checks passed.
The rows below carry the disposition instead of reopening or closing anything.
No private issue content or environment-specific evidence is reproduced here.

| Issue / state | Decision and input / matrix rows | Failure and delivered fix | Acceptance retained or explicitly transferred |
| --- | --- | --- | --- |
| [#3051](https://github.com/sympoies/nils-cli/issues/3051), closed | Canonical producer route and physical machine row; structured + local + exact recorded bytes; B02, B07, B12 | Raw continuation/provenance and missing busy correlation; #3205/#3400 delivered source, #3452 later extends mailbox/operator attestation. | Bootstrap/start, queued continuation, compaction/history, provider/terminal distinctions, invalid metadata no-effect, duplicate/unknown controls, exact bytes and generated trailing-space prevention stay identifiable. Remaining scheduler/native lifecycle acceptance is [#3448](https://github.com/sympoies/nils-cli/issues/3448), unchecked; this PR claims no new native result. |
| [#3180](https://github.com/sympoies/nils-cli/issues/3180), closed | Readable wake wording versus route constraints; local route and screen width; C01, C09 | Compact category letters obscure action; #3199 changed notice wording; later current-format retirement and command guidance supersede that intermediate split. | Source acceptance closed by [recorded disposition](https://github.com/sympoies/nils-cli/issues/3180#issuecomment-6570231422). The earlier dual-recognizer rollout criterion was explicitly replaced by the [first decision](https://github.com/sympoies/nils-cli/issues/3180#issuecomment-6560863372) and [later retirement decision](https://github.com/sympoies/nils-cli/issues/3180#issuecomment-6568718488), not silently satisfied; current producers/recognizers migrate together. Wording never permits replay of old unknown attempts. |
| [#3089](https://github.com/sympoies/nils-cli/issues/3089), closed | Exact notice transport supported before input? local capability, not screen similarity; B06 | Unsupported Claude terminal envelope could paste before failing; #3090 refuses before a new claim/load/paste/Enter. | [Closeout](https://github.com/sympoies/nils-cli/issues/3089#issuecomment-6625249535) records release acceptance. Keep short/long pre-input refusal, genuine earlier guards and consumed-key semantics; full delivery belongs to the native owner. |
| [#3002](https://github.com/sympoies/nils-cli/issues/3002), closed | Can trailing-space notice bytes be proved? structured module + bound row versus opaque screen; B06, B07 | Terminal renderer cannot prove the hidden exact body; #3090 containment and #3104/#3140 native route replace that proof source. | [Restated acceptance](https://github.com/sympoies/nils-cli/issues/3002#issuecomment-6561423170) deliberately keeps terminal notices refused. [#3448 comparison](https://github.com/sympoies/nils-cli/issues/3448#issuecomment-6754734648) records terminal no-input refusal and exact native trailing-space/final-LF, long and non-ASCII body/history checks; row checked. Cleanup inventory had a disclosed limit. No new run here. |
| [#2994](https://github.com/sympoies/nils-cli/issues/2994), closed | Supported notice delivery now or after busy queue? structured/local route and composer proof; B06, B07 | Codex-only support left Claude/terminal and busy queue gaps; #2997 queue/terminal support plus #3090 containment/#3104 native alternative. | [Closeout](https://github.com/sympoies/nils-cli/issues/2994#issuecomment-6625274774) cites native delivery under #3068 and transfers the trailing-space remainder to #3002. Keep one delivery after idle, no partial input on refusal and native/terminal capability distinctions. |
| [#3008](https://github.com/sympoies/nils-cli/issues/3008), closed | Should routine mail wake now? local destination policy/category/generation; C03 | Repeated routine interruptions; #3017 persists recipient-owned routine coalescing with urgent/service bypass and unknown-attempt window consumption. | Retain burst, bypass, restart, unchanged-default and identity/idempotency controls. [Closeout](https://github.com/sympoies/nils-cli/issues/3008#issuecomment-6625584039) explicitly waived the extra paired rollout check; do not report it as performed. Sender fields never set destination policy. |
| [#3114](https://github.com/sympoies/nils-cli/issues/3114), closed design | Caller command versus text intent; request input, not provider screen; D01 / R6 | Slash-name-plus-prose is ambiguous; merged [explicit-intent design #3389](owner-prompt-intent.md), not implementation. | Grammar/explicit field, typed refusals, affected owners, synthetic HTTP plan and consumer/design review stay in that spec. [Final disposition](https://github.com/sympoies/nils-cli/issues/3114#issuecomment-6838836549) leaves implementation unscheduled with visible-action sign-off/release fences as gates. It differs from earlier body status; do not infer implementation authority from the older wording. |
| [#2922](https://github.com/sympoies/nils-cli/issues/2922), closed | Is dim hint plus three unstyled spaces empty? screen styling; A01 | Empty Codex draft after Backspace falsely refuses; #2924 adds parser and idle/working-path controls. | [#3448 owner disposition](https://github.com/sympoies/nils-cli/issues/3448#issuecomment-6733124373) accepts released fixture evidence after three bounded live trials did not reproduce the exact form; checked by decision, not an exact-state live PASS. Keep real/wrapped draft, dialog, unobservable and once-only history controls; reopen on failing evidence. |
| [#2691](https://github.com/sympoies/nils-cli/issues/2691), open | May HTTP clear the live draft atomically with prompt admission? local expected incarnation, structured idle/attention, screen composer and keyed write evidence; A03, A07, R2 | HTTP read-then-key write lacks one guarded serialization/receipt boundary; saved-draft discard is not live composer clearing. No merged fix establishes this acceptance. | Preserve all five criteria: Claude/Codex idle one-line clear with observed empty/readback and next prompt once; working/attention/stale/unobservable pre-input refusal; late-state recheck and edit-key allowlist excluding Ctrl+C; clear/prompt serialization and no blind unknown/key replay; documented route/schema/codes and nullable composer result. Existing raw-send behavior remains a separate operation; no new compatibility alias is proposed. |
| [#3126](https://github.com/sympoies/nils-cli/issues/3126), closed acceptance | Does released behavior match the guarded contract? actual provider/terminal and fault evidence; A05, A12, B01 | Synthetic tests cannot establish all native fault/compaction behavior; no new source fix is specified. | [Transfer](https://github.com/sympoies/nils-cli/issues/3126#issuecomment-6838257807) leaves Codex and terminal long ASCII/final-space, CJK, replay/draft, fault and compaction-boundary runs in [#3448 comments](https://github.com/sympoies/nils-cli/issues/3448#issuecomment-6838251504), pending. Claude acceptance elsewhere is not a substitute. |

## Why the causes are shared or distinct

**R1: information loss at a rendered boundary.** A01–A10 and A13–A15 show
multiple consumers deciding empty, exact-owned or safe-to-clear from a lossy
render. A new layout can change all three decisions. The useful shared unit is
an operation-bound observation with an explicit capability and typed unknown,
not another string recognizer. The terminal's remaining observation race must
continue to report `atomic: false`.

**R2: evidence and effects advance at different times.** A06–A12, B01–B02,
B07, B12 and C04 distinguish an observation, a durable claim, text release,
acknowledgment and row confirmation. Delayed rendering and lost native ack have
the same retry-safety problem even though their proof channels differ.
Authenticated positive evidence can upgrade an existing receipt; absence cannot
create permission to resend. A notice's machine-origin proof is stronger than
an ordinary prompt's delivery proof and cannot be collapsed into it.

**R3: scheduling is not provider activity.** C02–C09 read unread generations,
recipient policy, quiescence and route capability. An advisory work claim does
not mean a turn is active (#3443), and a queued generation does not prove an
attempt was released (#3490). A structured provider can still suffer these
failures. Reuse the existing generation/CAS owner; a better composer parser
cannot repair its scheduling semantics.

**R4: provenance/selection is not confirmation.** B12–B15 use the same notice
claim and physical-row join, but a display echo may be flattened or truncated.
Those masking rules may hide a receipt-bound echo; they cannot prove exact
input, confer authority on a human header or authorize a send. This explains
why native sends did not eliminate the later preview fixes.

**R5 and R6 have independent counterexamples.** A valid structured message can
still fail on module load shape/version, lock ordering, external tmux behavior,
account binding or archive capacity. Likewise, no provider-state source can
discover whether a person intended slash text as a command. Keep provider
adapter qualification and the existing explicit-intent design separate.

Boundaries examined but not folded into implementation here: mailbox
storage/quota/federation (D04), account/auth recovery, image-content projection,
mailbox read-event display, broad turn-state changes and runtime replacement.
They may supply inputs or controls; their own issue acceptance stays with them.
The active [#3215](https://github.com/sympoies/nils-cli/issues/3215) and
[#3592](https://github.com/sympoies/nils-cli/issues/3592) owners retain their
native-refusal and Claude-state slices. Their unmerged work is not evidence of
behavior at this pin.

[#2887's reopened acceptance](https://github.com/sympoies/nils-cli/issues/2887#issuecomment-6831841737)
separately owns a documented terminal bound for pre-attempt
`recipient-composer-unobservable` on Codex records whose provider binding is
not captured yet and that have no structured prompt route, with a sender-readable
reason. A terminal-started record can later obtain its binding from a hook or
session metadata. The attempt-path fixes #3345/#3618 did not satisfy that acceptance.

[PR #3650](https://github.com/sympoies/nils-cli/pull/3650) delivered the bound in
[`cb325526bd3d41217115eb2072bbda9326c7ce4b`](https://github.com/sympoies/nils-cli/commit/cb325526bd3d41217115eb2072bbda9326c7ce4b),
after the source pin: the fifth consecutive such refusal of one generation
settles it as `undeliverable` with reason `recipient-composer-unobservable`;
the first four retain the 1 s retry. Another recorded reason resets the count;
a later message starts a new generation. Draft refusals,
`recipient-working` / `provider-not-ready` remain transient, bound
Console-started controls must keep working, and resident acceptance remains
with [#3448](https://github.com/sympoies/nils-cli/issues/3448).
This post-pin delivery does not change the shared design's proposed status.

The size of the five-attempt bound (about 4 s at the retry cadence) remains a
[documented non-blocking follow-up, F2 on #2887](https://github.com/sympoies/nils-cli/issues/2887#issuecomment-6847858442).
This note records the delivered bound without selecting a new size or policy.

## Proposed shared design

Use the existing send guard and notice/receipt owners. Do not add a new delivery
service, generic transport registry, duplicate codec or state store. The real
consumers are operator guarded sends, direct owner prompts, scheduler notices
and controller/hook wakes; sharing is justified only at their common proof
boundaries. The lead selects any later implementation slices.

### One operation-bound observation contract

At each input boundary bind evidence to the existing managed session/runtime
generation, provider conversation/thread, applicable account identity, exact
payload digest/length, attempt key or notification generation and deadline.
Evidence from another incarnation, turn or payload cannot authorize a mutation.
Keep turn/activity, composer, input acceptance and machine provenance as separate
facts. Never derive one from another because they are simultaneously visible.

The internal composer result should distinguish empty, observed human draft,
owned input for this attempt, and unobservable with a fixed reason. Source,
identity, observation stage and existing time bound accompany the result.
Owned input requires the current attempt's ownership proof; a matching header,
old paste token, retained saved draft or neighboring row cannot supply it.
Reuse the current `Unobserved` reason vocabulary and existing identity checks.
A small helper in the existing guard subtree can carry these facts for direct
prompt and notice consumers; do not introduce a second public schema.

For a supported attached Claude **notice**, use the authenticated, fresh
module authorization's `prompt.read` result (`box_empty`, cursor zero) as the
composer fact at its release boundary. That module already reads the actual
draft and rechecks it before submit. Proposed change: remove the redundant
screen-based empty verdict from this supported native notice branch only,
while retaining runtime/idle/ingress/account/claim checks and the module's
independent empty check. Bind the response to the current offer/nonce/module
identity; a cached readiness file or a new standalone boolean is insufficient.
This is the concrete decision that can stop depending on screen text.

This reuse serves operator, scheduler and mailbox notices. It must first pass
the synthetic authorization/draft race controls below. It is not permission to
turn the notice-only module into a general prompt transport or to treat a busy
provider as eligible. The ordinary Claude prompt route has no such accepted
module contract and keeps its current guarded terminal proof.

Codex app-server idle and client-id acceptance remain structured. Its attached
TUI's unsent draft is not exposed by the app-server contract at this pin, so
empty-composer policy still needs the frozen pane reader. Ordinary terminal
pastes, owner command UI and #2691 live draft clearing likewise retain their
bounded existing proof unless a separately demonstrated structured source
exists. Unknown render forms stay typed refusals. Do not widen the corpus
classifier merely to make another layout pass, or equate a native non-paste
transport with an empty human composer.

### One effect vocabulary, existing durable owners

Normalize the internal result at existing adapter boundaries into these facts;
map to current public receipts/codes without changing their meaning:

| State | Required evidence | Permitted next action |
| --- | --- | --- |
| Prepared / not offered | Identity/capability/idle/composer checked; durable notification generation not consumed | Defer that queued generation under its existing retry policy; do not claim an unsupported route succeeded. |
| Consumed / release possible | Durable claim or notification CAS completed before any text can reach the provider | At most one dispatch for that attempt; crash or lost response keeps it consumed. |
| Acknowledged, unconfirmed | Provider acknowledgment or accepted Enter, without the route's complete positive proof | Observe under original fences; preserve unknown and any residue; never repaste or start another native turn. |
| Confirmed delivery | Existing exact input/turn or client-id proof; notice also needs its exact bound physical row | Publish the existing submitted/queued/command-specific result; no extra dispatch. |
| Definite pre-release refusal | Proven no release at that boundary, with actual claim status retained | Return typed refusal; only an unconsumed notification generation remains eligible for deferred retry. |
| Unknown after consumption | Missing, ambiguous, late, replaced or timed-out proof | Retain durable dedupe and unknown outcome; positive reconciliation only where already supported. |

These are design terms, not replacement persisted enums. A durable key may
already be consumed even when a final admission recheck refuses before input;
no mapping may erase that fact. A control-queue timeout alone cannot prove the
receiver never wrote. Preserve the receiver's current write-time identity,
generation and deadline fences. A later lookup may have a bounded observation
budget of its own, but never renews the original dispatch authorization or
permits a second mutation.

Refactor only duplicate preoffer/consumed-result mapping between the controller
native/terminal notice branches and guarded delivery. Keep durable claims in
`send_guard/key.rs`, prompt receipts in their current store, and mailbox
transitions in `coordination/notification.rs` and `quiescence_attempt.rs`.
Share pure evidence/result classification, not lock ownership or persisted
records. Native lock release remains at the current acknowledgment/write hook
boundary, with identity revalidation after reacquisition and no stale record
write-back. Scheduler admission retains its own exact failed-predecessor,
activity/cause and non-idle recovery authorization.

### Route selection and wake state

Resolve capability and caller purpose **before** release, then keep that
attempt on its selected route. Notice-module failure after consumption must
not become a terminal/plain wake. Preserve current #3527 behavior: known Claude
launch/configuration absence can select the guarded one-line mailbox text
route; an attached module whose readiness/authorization is uncertain remains
on the native path and deferred while unconsumed. This is a supported current
route distinction, not an old-format reader or an after-failure retry path.
Operator exact-envelope notices remain unsupported on the Claude terminal route.

| Recipient/attempt state | Input | Required decision |
| --- | --- | --- |
| Idle, own advisory claim only | Verified provider idle plus local quiescence | May wake; claim alone does not block. |
| Idle, active/uncertain operation or restart/auth fence | Structured/local guard state | Defer before release with bounded blocking condition. |
| Working/needs-input Codex checkpoint | Exact authoritative active turn plus in-turn quiescence | Preserve existing steer policy and claim fence; no general busy bypass. |
| Occupied or unreadable human composer | Authoritative native notice read where supported, otherwise frozen screen | Preserve the draft; use existing stale-draft policy only on routes that already own it. |
| Codex records whose provider binding is not captured yet, pre-attempt composer unobservable | `recipient-composer-unobservable`, no usable provider binding or structured prompt route | [#2887](https://github.com/sympoies/nils-cli/issues/2887#issuecomment-6831841737) owns bounded terminal settlement with a sender-readable reason. [#3650](https://github.com/sympoies/nils-cli/pull/3650), merged as `cb325526bd3d41217115eb2072bbda9326c7ce4b` after the pin, settles `undeliverable` with that reason on the fifth consecutive refusal of one generation; earlier refusals retry at 1 s, and a later message starts a new generation. Keep draft refusals, `recipient-working` / `provider-not-ready` transient and bound Console-started controls intact. The bound's size remains [F2's follow-up](https://github.com/sympoies/nils-cli/issues/2887#issuecomment-6847858442). |
| Routine coalesced generation before due time | Recipient policy, unread generation and saved window | Stay queued; urgent/service bypass remains destination-controlled. |
| Unoffered native receiver timeout | Authenticated release/CAS fact | Same unconsumed generation may retry after the receipt-owned backoff. |
| Attempted generation, unknown result | Durable consumed attempt | Keep unknown, even if no row is yet visible; newer unread mail follows existing generation recovery. |
| Inbox drained or incarnation replaced | Canonical unread state / identity | Resolve drained work or refuse the replaced target; no stale wake. |

Retain one canonical readable command-bearing wake producer and its trusted
executable/recipient validation. Hook-only delivery still claims at its own
model boundary. It is not evidence of a daemon turn or a mailbox-read display
event. Preserve format identities for immutable attempted evidence without
reviving retired producers, aliases or old-format dispatch.

## Contract tests required before implementation

This PR changes no test. The cases below are acceptance requirements for later
slices, not claims of new red/green or native coverage. Reuse the existing
stable fixtures and add a case only for a distinct uncovered behavior.

| Contract | Existing owner to extend | Observable assertions / distinguishing negative control |
| --- | --- | --- |
| T1: authoritative native composer fact | `send/claude_notice/module_contract.rs`, bridge/refusal tests and `tests/session_notices.rs` | An authenticated current empty authorization permits exactly one notice even if a synthetic screen is unfamiliar; a real native draft, wrong nonce/module/runtime or changed cursor refuses before text release. Scheduler and operator/mailbox variants retain their distinct eligibility. |
| T2: post-authorization human edit | Existing Claude notice module/receiver fixture | The module's second `prompt.read` observes the edit: no submit or clear; consumed unknown stays consumed and is not reassigned to text transport. Native acceptance alone never deletes the human draft. |
| T3: frozen terminal corpus | `send_submit/composer/corpus.rs`, `real-pane-corpus.json`, `tests/guarded_send/live_composer.rs` | Pin exact/direct verdict and fixed reason for dim hints/padding, OSC-8, clipped border, version suggestion, choice/dialog, wrapped/CJK/trailing-space and opaque token cases already represented. Named edits change one feature; an unknown form refuses without input. |
| T4: one effect classification across callers | `serve/tests/mailbox_notice_terminal.rs`, `mailbox_controller.rs`, `terminal_notification/native_notice.rs`, guarded-send fixtures | Inject preoffer refusal, release-time CAS failure, postclaim timeout, lost result and late positive proof at existing boundaries. Assert generation/key consumption, retry eligibility and provider mutation count separately. A control-queue response timeout cannot become a fresh attempt. |
| T5: native identity and input proof | `codex_app_server/guarded/confirmation.rs`, `serve/tests/prompt_receipts/`, `native_busy_reply.rs` | Reuse unique client-id/exact-text start/steer positives. Missing/duplicate id, sibling text, changed account/thread/runtime, delayed row and interrupted confirmation preserve unknown and issue no second dispatch. Human exact-text copy cannot become a notice. |
| T6: wake scheduling/restart | `coordination/notification_policy_tests.rs`, `notification_coalescing_tests.rs`, `broker_notification_resolution_tests.rs`, `serve/tests/mailbox_resume.rs` | Routine burst/window, urgent bypass, own claim versus active operation, drained inbox, restart, newer generation behind old unknown, and receipt-owned retry deadline preserve one intended wake and correct pending work. Reuse existing cases rather than duplicate their implementation. |
| T7: route capability and no downgrade | Existing `mailbox_notice_fallback.rs`, terminal/native notice and hook reminder fixtures | Known module absence selects exactly one guarded one-line text attempt with trusted recipient read action; attached-but-unready never selects text. Writers-disabled Codex hook remains for controller. No native postclaim failure switches transport. |
| T8: live draft-clear boundary (#2691 only) | New focused HTTP fixture beside `serve/tests/prompt_receipts/` using existing synthetic tmux/control recorder | Idle Claude/Codex one-line draft clears, observed empty readback, next prompt once; stale incarnation, late activity/attention, unreadable or working state and disallowed keys cause zero effects. Race clear against prompt, persist key replay/unknown and nullable composer result. Saved-draft discard must not satisfy this test. |
| T9: notice row origin versus preview echo | Existing native-notice paging, launch-row and prompt-preview tests | Reuse exact physical-row/hash/client-id proof across forward/reverse/incremental/live readers. Flattened/truncated bound echo may be hidden but cannot confirm input; unbound human header and same text outside proof stay human. |

For every production bug/contract change: record a meaningful failing test at
the actual boundary before the first production edit using the repository's
test-first evidence workflow. T1's red should be an otherwise eligible,
authenticated notice refused only by the redundant screen verdict, not an
unsupported provider or fixture setup failure. T4's red must show an actual
wrong consumed/retry classification. If those cases already pass, do not invent
a production fix or duplicate tests to obtain a red result.

Corpus policy: use synthetic text or a sanitized real capture and explicitly
named derivations only. Preserve escape bytes, cell widths, cursor coordinates,
provider/build provenance and the exact expected outcome; replace all personal
content, titles and paths with generic same-shape values. Corpus data cannot
prove support for an unrepresented provider build. No user transcript, private
session identifier, account data, shared GUI or live device belongs in tests.
The normal unit/integration suite must consume the corpus without live accounts
or an opt-in manual test flag. Existing corpus coverage is already delivered
by #3216; extend it only for a valid new distinction.

## Affected modules and migration boundary

Paths here are relative to `crates/agent-session` unless otherwise stated.
These are proposed later touchpoints; this PR edits documentation only.

| Responsibility | Existing files / affected caller set |
| --- | --- |
| Composer evidence boundary | `src/send_guard.rs`, `src/send_guard/composer.rs`, `pasted.rs`, `ownership.rs`, `clear.rs`, `placeholder.rs`; put new shared logic in a focused child of the guard owner and migrate direct-prompt/native-notice callers together. |
| Native Claude notice authorization | `src/send/claude_notice.rs`, `claude_notice/bridge.rs`, `refusal.rs`, `register.js`, `runtime.rs`; reuse bound authorize/result messages and the second native draft check. A protocol change, if required, migrates module, bridge, validator and all fixtures together. |
| Submission and wake result mapping | `src/send.rs`, `src/send/native.rs`, `daemon_notice.rs`, `src/serve/terminal_notification/{mailbox_dispatch,native_notice,attested}.rs`, `src/coordination/{notification,notification_observation,quiescence_attempt}.rs`. Keep CAS, durable records and route-specific transport proof with their current owners. |
| Native write and receipt consumers | `src/codex_app_server/{guarded,mailbox_notice,prompt_submit}.rs`, `guarded/confirmation.rs`, `src/serve/prompt_receipts/{client_message,reconcile,store}.rs`, `src/send/codex_notice.rs`; preserve current protocol ids, write fences and no-replay behavior. Change only a demonstrated duplicate mapping or affected caller. |
| Corpus and behavioral tests | Existing files named in T1–T9, plus `src/send_submit/composer/{corpus.rs,real-pane-corpus.json}` and existing native capture fixtures. No second fixture framework. |
| HTTP live draft clear | #2691's later focused handler/body/receipt module under `src/serve/`; serialize with the existing prompt input boundary and document exact edit-key/request-key semantics before implementation. Existing raw `/send` is not rebranded as guarded clear. |
| Canonical docs and supported callers | `docs/specs/send-v1.md`, `session-notices-v1.md`, `coordination-mailbox-v1.md`, `serve-prompt-v1.md`, `serve-terminal-v1.md`, lifecycle runbook, and `crates/agent-hook/src/dsh_policy/mailbox_reminder.rs` only if its actual notice contract changes. Inventory every producer/consumer at the selected slice's base. |

Baselined `src/lib.rs` and `src/serve.rs` cannot grow, even by a module line.
Any necessary registration must move existing code into an owning module and
reduce the baseline file; no broad crate split belongs to this work. A module
listed as a touchpoint is not authorization to rewrite it without an observed
delta. Coordinate with the active source owners before any production edit.

Keep the current repository contract only: no new compatibility switch, alias,
dual reader/writer, generic fallback or old-format replay. Migrate all affected
supported callers and fixtures in a single accepted change. Retained signed
claims/attestations are immutable provenance, not permission to emit old forms.
If deployed embedded modules or CLI/controller/broker/hook readers need matching
binaries, name the reinstall/restart and one-shot state step in that slice's
handoff; do not solve fleet timing with another runtime branch. The separate
owner-intent design's release-owner gates remain with that design; this PR
neither supplies them nor creates another overlap.

Untouched: mailbox read events #3171, account switching/auth policy, broad
`lib.rs`/`serve.rs` splitting, gate speed/build policy, provider version-range
widening, command grammar/denylist/login/quarantine policy, consumer UI and
release/deployment configuration. Existing stale-draft thresholds, human-draft
preservation, attention/ingress fences, scheduler recovery eligibility and
transport-specific deadlines stay authoritative. No live writer enablement,
shared-resource probing or issue acceptance closure follows from this note.

## Review and follow-through

Step 1 is complete when the matrix, evidence-backed grouping, proposed shared
boundaries, future test requirements and untouched scope are reviewable in a
docs-only PR linked from #3186. Run the repository docs-only entrypoint; no Rust
build is needed. A docs proposal needs no product red record. Required CI and
design review remain separate delivery gates.

The lead decides implementation slices after review. Recommended order is to
confirm the native-notice composer boundary (T1/T2), then extract only a proven
duplicate effect/result mapping (T4) with existing wake policy tests intact.
Treat #2691 as a separate HTTP mutation contract using those shared facts,
not an incidental widening of send. Preserve the independent owner-intent and
active startup/state/refusal work. Do not rerun already accepted #3002/#2922
checks merely because they appear in this matrix.

Native/provider acceptance still pending for #3051 and #3126 stays with the
scheduled tester via #3448: disposable start/queue/compaction/history and
Codex/terminal fault paths as carried above. This analysis supplies no new live
PASS, installed-binary claim, release, deployment or guarantee of eventual wake.
