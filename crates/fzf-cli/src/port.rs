use crate::{fzf, kill, util};

pub fn run(args: &[String]) -> i32 {
    let flags = kill::parse_kill_flags(args);
    let query = util::join_args(&flags.rest);

    if util::cmd_exists("lsof") {
        let lsof_out = match util::run_capture("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"]) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("{err:#}");
                return 1;
            }
        };
        let mut lines = lsof_out.lines();
        let _ = lines.next();
        let input = lines.collect::<Vec<_>>().join("\n");

        let preview = r#"printf "%s\n" {} | awk '{
  cmd = $1; pid = $2; user = $3;
  proto = "?"; name = "";
  for (i=1; i<=NF; i++) if ($i == "TCP" || $i == "UDP") { proto = $i; break }
  for (i=NF; i>=1; i--) if (index($i, ":") > 0) { name = $i; break }

  printf "🔭 PORT\n%s\n\n", name;
  printf "🌐 PROTO\n%s\n\n", proto;
  printf "📦 CMD\n%s\n\n", cmd;
  printf "👤 USER\n%s\n\n", user;
  printf "🔢 PID\n%s\n\n", pid;

  if (pid ~ /^[0-9]+$/) {
    printf "lsof -p %s\n\n", pid;
    system("lsof -nP -p " pid " 2>/dev/null | sed 1d | head -n 80");
  }
}'"#;

        let args_vec: Vec<String> = vec![
            "-m".to_string(),
            "--prompt".to_string(),
            "🔌 Port > ".to_string(),
            "--query".to_string(),
            query,
            "--preview-window=right:50%:wrap".to_string(),
            "--preview".to_string(),
            preview.to_string(),
        ];
        let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();

        let (code, selected) = match fzf::run_lines(&format!("{input}\n"), &args_ref, &[]) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("{err:#}");
                return 1;
            }
        };
        if code != 0 {
            return 0;
        }

        let mut pids: Vec<String> = selected
            .iter()
            .filter_map(|line| line.split_whitespace().nth(1).map(|s| s.to_string()))
            .collect();
        pids.sort();
        pids.dedup();

        match kill::kill_flow(&pids, flags.kill_now, flags.force_kill) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("{err:#}");
                1
            }
        }
    } else {
        let netstat_out = match util::run_capture("netstat", &["-anv"]) {
            Ok(v) => v,
            Err(_) => return 0,
        };
        let filtered = netstat_out
            .lines()
            .filter(|l| l.trim_start().starts_with("tcp") || l.trim_start().starts_with("udp"))
            .collect::<Vec<_>>()
            .join("\n");

        let args_vec: Vec<String> = vec![
            "-m".to_string(),
            "--prompt".to_string(),
            "🔌 Port > ".to_string(),
            "--query".to_string(),
            query,
            "--preview-window=right:50%:wrap".to_string(),
            "--preview".to_string(),
            "printf \"%s\\n\\n(netstat view; no lsof PID info)\\n\" {}".to_string(),
        ];
        let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();

        let (_code, _selected) = match fzf::run_lines(&format!("{filtered}\n"), &args_ref, &[]) {
            Ok(v) => v,
            Err(_) => return 0,
        };

        0
    }
}
