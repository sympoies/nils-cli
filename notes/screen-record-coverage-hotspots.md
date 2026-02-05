# screen-record coverage hotspots

Data source: `target/coverage/screen-record.lcov.info`

## Baseline totals (current lcov)

- Lines: `882/1178` (`74.87%`, miss `296`)
- Functions: `89/118` (`75.42%`, miss `29`)
- Branches: `0/0` (`N/A` in current lcov export)

## Top uncovered files (exact counts)

| Rank | File | Lines hit/total | Miss lines | Functions hit/total | Miss functions |
| --- | --- | --- | ---: | --- | ---: |
| 1 | `crates/screen-record/src/run.rs` | `460/730` (`63.01%`) | 270 | `39/62` (`62.90%`) | 23 |
| 2 | `crates/screen-record/src/test_mode.rs` | `131/148` (`88.51%`) | 17 | `9/13` (`69.23%`) | 4 |
| 3 | `crates/screen-record/src/select.rs` | `222/228` (`97.37%`) | 6 | `28/29` (`96.55%`) | 1 |
| 4 | `crates/screen-record/src/error.rs` | `19/22` (`86.36%`) | 3 | `5/6` (`83.33%`) | 1 |

## Top uncovered functions (0-hit declarations)

`run.rs` (8 functions with `FNDA=0`):
- `preflight` (`run.rs:226`)
- `request_permission` (`run.rs:252`)
- `screenshot_portal` (`run.rs:337`)
- `record_portal` (`run.rs:461`)
- `resolve_portal_screenshot_output` (`run.rs:684`)
- `format_label` (`run.rs:991`)
- `ensure_linux_x11_only_mode_allowed` (`run.rs:1002`)
- `ensure_linux_x11_selectors_allowed` (`run.rs:1022`)

Other file:
- `unsupported_platform` (`crates/screen-record/src/error.rs:14`)

## Sprint 1 target branches in `run.rs` (Task 1.2 + 1.3 scope)

Current branch metric fallback: use 0-hit `DA` lines (because `BRF/BRH=0/0`).

| Target branch group | Lines | Current 0-hit DA lines |
| --- | --- | ---: |
| `validate_portal_flag_usage` (portal mode + non-Linux rejection) | 524-551 | 10 |
| `validate_record_args` | 553-594 | 4 |
| `validate_screenshot_args` | 596-639 | 3 |
| `ensure_no_recording_flags` | 641-662 | 3 |
| `resolve_output_path` | 664-682 | 4 |
| `resolve_portal_screenshot_output` | 684-751 | 51 |
| `resolve_screenshot_output` | 753-822 | 20 |
| `resolve_image_format` | 824-869 | 4 |
| `screenshot_timestamp` | 887-899 | 5 |
| `sanitize_filename_segment` | 917-960 | 16 |
| `resolve_container` | 962-989 | 10 |
| **Sprint 1 target total** |  | **130** |

### Expected delta / checkpoint

- Upper bound (if Sprint 1 target lines are fully covered):
  - `run.rs`: `460/730 -> 590/730` (`63.01% -> 80.82%`, `+17.81pp`)
  - crate lines: `882/1178 -> 1012/1178` (`74.87% -> 85.91%`, `+11.04pp`)
- Sprint 1 checkpoint (50% of target lines closed, i.e. `+65` lines):
  - `run.rs` >= `525/730` (`71.92%`)
  - crate lines >= `947/1178` (`80.39%`)

## Repro commands

```bash
# 1) Baseline totals
awk -F: '
  $1=="LH"{lh+=$2} $1=="LF"{lf+=$2}
  $1=="FNH"{fnh+=$2} $1=="FNF"{fnf+=$2}
  $1=="BRH"{brh+=$2} $1=="BRF"{brf+=$2}
  END {
    printf "lines %d/%d %.2f%%\\n", lh, lf, (lf?100*lh/lf:0);
    printf "functions %d/%d %.2f%%\\n", fnh, fnf, (fnf?100*fnh/fnf:0);
    printf "branches %d/%d %.2f%%\\n", brh, brf, (brf?100*brh/brf:0);
  }
' target/coverage/screen-record.lcov.info

# 2) Top uncovered files
awk -F: '
  $1=="SF"{sf=$2}
  $1=="LF"{lf[sf]=$2} $1=="LH"{lh[sf]=$2}
  $1=="FNF"{fnf[sf]=$2} $1=="FNH"{fnh[sf]=$2}
  $1=="end_of_record"{
    ul=lf[sf]-lh[sf]; uf=fnf[sf]-fnh[sf];
    printf "%d\\t%d/%d\\t%d/%d\\t%s\\n", ul, lh[sf], lf[sf], fnh[sf], fnf[sf], sf;
  }
' target/coverage/screen-record.lcov.info | sort -t$'\\t' -k1,1nr

# 3) Sprint 1 target DA-miss count in run.rs
awk -F: '
  $1=="SF"{in_run=($2 ~ /crates\\/screen-record\\/src\\/run.rs$/)}
  in_run && $1=="DA"{
    split($2,a,","); line=a[1]+0; hit=a[2]+0;
    if (hit==0) {
      if (line>=524 && line<=551) c["validate_portal_flag_usage"]++;
      if (line>=553 && line<=594) c["validate_record_args"]++;
      if (line>=596 && line<=639) c["validate_screenshot_args"]++;
      if (line>=641 && line<=662) c["ensure_no_recording_flags"]++;
      if (line>=664 && line<=682) c["resolve_output_path"]++;
      if (line>=684 && line<=751) c["resolve_portal_screenshot_output"]++;
      if (line>=753 && line<=822) c["resolve_screenshot_output"]++;
      if (line>=824 && line<=869) c["resolve_image_format"]++;
      if (line>=887 && line<=899) c["screenshot_timestamp"]++;
      if (line>=917 && line<=960) c["sanitize_filename_segment"]++;
      if (line>=962 && line<=989) c["resolve_container"]++;
    }
  }
  END {
    total=0;
    for (k in c) { print k, c[k]; total+=c[k]; }
    print "TOTAL", total;
  }
' target/coverage/screen-record.lcov.info
```

## Post-sprint snapshot (after Sprint 1-3 test additions)

- `scripts/ci/coverage-summary.sh target/coverage/screen-record.lcov.info`
  - Total line coverage: `1157/1371` (`84.39%`)
- `run.rs` unique line coverage (DA dedup by file+line):
  - `734/898` (`81.74%`)
- crate `src/` unique line coverage (DA dedup by file+line):
  - `1144/1328` (`86.14%`)

This meets the plan checkpoint targets:
- crate >= `82.00%`
- `run.rs` >= `78.00%`

### Remaining misses intentionally deferred

Top residual missed lines are concentrated in platform/branch-specific paths:
- `crates/screen-record/src/run.rs` (`164` missed DA lines)
- `crates/screen-record/src/test_mode.rs` (`13` missed DA lines)
- `crates/screen-record/src/select.rs` (`4` missed DA lines)
- `crates/screen-record/src/error.rs` (`3` missed DA lines)

These remaining misses are mostly in OS-gated branches and low-risk fallbacks; deferred to avoid
overfitting tests to platform internals in this PR.
