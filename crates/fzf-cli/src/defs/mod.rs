mod block_preview;
mod cache;
mod commands;
mod index;

pub fn run_env(args: &[String]) -> i32 {
    commands::run_env(args)
}

pub fn run_alias(args: &[String]) -> i32 {
    commands::run_alias(args)
}

pub fn run_function(args: &[String]) -> i32 {
    commands::run_function(args)
}

pub fn run_def(args: &[String]) -> i32 {
    commands::run_def(args)
}
