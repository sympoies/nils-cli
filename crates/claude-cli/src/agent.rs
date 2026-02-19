use claude_core::exec;

pub fn prompt(prompt_args: &[String]) -> i32 {
    let prompt = prompt_args.join(" ");
    if prompt.trim().is_empty() {
        eprintln!("claude-cli agent prompt: missing prompt");
        return 64;
    }

    run_execute(prompt.as_str())
}

pub fn advice(question_args: &[String]) -> i32 {
    let question = question_args.join(" ");
    if question.trim().is_empty() {
        eprintln!("claude-cli agent advice: missing question");
        return 64;
    }

    let task = format!("advice: {}", question.trim());
    run_execute(task.as_str())
}

pub fn knowledge(concept_args: &[String]) -> i32 {
    let concept = concept_args.join(" ");
    if concept.trim().is_empty() {
        eprintln!("claude-cli agent knowledge: missing concept");
        return 64;
    }

    let task = format!("knowledge: {}", concept.trim());
    run_execute(task.as_str())
}

fn run_execute(task: &str) -> i32 {
    match exec::execute_task(task, None, None) {
        Ok(result) => {
            println!("{}", result.stdout);
            if !result.stderr.is_empty() {
                eprintln!("{}", result.stderr);
            }
            0
        }
        Err(error) => {
            eprintln!("claude-cli agent: {} ({})", error.message, error.code);
            1
        }
    }
}
