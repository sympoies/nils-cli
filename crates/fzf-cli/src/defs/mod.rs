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

#[cfg(test)]
mod tests {
    use super::{run_alias, run_def, run_env, run_function};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn wrapper_entrypoints_delegate_and_return_block_preview_error_without_delims() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "FZF_DEF_DELIM", "");
        let _guard_end = EnvGuard::set(&lock, "FZF_DEF_DELIM_END", "");

        let args = vec!["query".to_string()];
        assert_eq!(run_env(&args), 1);
        assert_eq!(run_alias(&args), 1);
        assert_eq!(run_function(&args), 1);
        assert_eq!(run_def(&args), 1);
    }
}
