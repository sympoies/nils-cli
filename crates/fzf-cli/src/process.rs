use crate::{fzf, kill, util};

pub fn run(args: &[String]) -> i32 {
    let flags = kill::parse_kill_flags(args);
    let query = util::join_args(&flags.rest);

    let ps_out = match util::run_capture(
        "ps",
        &["-eo", "user,pid,ppid,pcpu,pmem,stat,lstart,time,args"],
    ) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    let mut lines = ps_out.lines();
    let _ = lines.next();
    let input = lines.collect::<Vec<_>>().join("\n");

    let args_vec: Vec<String> = vec![
        "-m".to_string(),
        "--query".to_string(),
        query,
        "--preview-window=right:30%:wrap".to_string(),
        "--preview".to_string(),
        "printf \"%s\\n\" {}".to_string(),
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
}
