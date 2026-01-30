mod codex;
mod commit;
mod git;
mod staged_context;
mod usage;

pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    usage::dispatch(&args)
}
