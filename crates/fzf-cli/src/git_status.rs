use crate::{fzf, util};

pub fn run(args: &[String]) -> i32 {
    if !is_git_repo() {
        eprintln!("❌ Not inside a Git repository. Aborting.");
        return 1;
    }

    let query = util::join_args(args);
    let status_out = match util::run_capture("git", &["status", "-s"]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    let preview = r#"bash -c '
      line=$1
      path=$(printf "%s" "$line" | cut -c4-)

      case "$path" in
        *" -> "*) path=$(printf "%s" "$path" | sed -E "s/^.* -> //") ;;
      esac

      first=$(printf "%s" "$path" | cut -c1)
      last=$(printf "%s" "$path" | tail -c 1)
      if [[ "$first" == "\"" && "$last" == "\"" ]]; then
        raw=$(printf "%s" "$path" | sed -E "s/^\"//; s/\"$//")
        path=$(printf "%b" "$raw")
      fi

      if git ls-files --others --exclude-standard -- "$path" | grep -q .; then
        printf "%s\n" "--- UNTRACKED ---"
        git diff --color=always --no-index /dev/null -- "$path" 2>/dev/null || true
        exit 0
      fi

      printed=0

      if ! git diff --cached --quiet -- "$path" >/dev/null 2>&1; then
        printf "%s\n" "--- STAGED ---"
        git diff --color=always --cached -- "$path"
        printed=1
      fi

      if ! git diff --quiet -- "$path" >/dev/null 2>&1; then
        if [ "$printed" -eq 1 ]; then
          printf "\n"
        fi
        printf "%s\n" "--- UNSTAGED ---"
        git diff --color=always -- "$path"
        printed=1
      fi

      if [ "$printed" -eq 0 ]; then
        printf "%s\n" "(no diff)"
      fi
    ' -- {}"#;

    let args_vec: Vec<String> = vec![
        "--query".to_string(),
        query,
        "--preview".to_string(),
        preview.to_string(),
        "--bind=ctrl-j:preview-down".to_string(),
        "--bind=ctrl-k:preview-up".to_string(),
    ];

    let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
    let (code, _lines) = match fzf::run_lines(&status_out, &args_ref, &[]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    if code != 0 {
        return 0;
    }

    0
}

fn is_git_repo() -> bool {
    util::run_output("git", &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}
