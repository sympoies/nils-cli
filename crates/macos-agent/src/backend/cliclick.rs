use crate::backend::process::{map_failure, ProcessRequest, ProcessRunner};
use crate::cli::MouseButton;
use crate::error::CliError;

pub fn click(
    runner: &dyn ProcessRunner,
    x: i32,
    y: i32,
    button: MouseButton,
    count: u8,
    timeout_ms: u64,
) -> Result<(), CliError> {
    if count == 0 {
        return Err(CliError::usage("--count must be at least 1"));
    }

    let action = match button {
        MouseButton::Left => "c",
        MouseButton::Right => "rc",
        MouseButton::Middle => "mc",
    };

    let mut args = Vec::with_capacity(count as usize);
    for _ in 0..count {
        args.push(format!("{action}:{x},{y}"));
    }

    let request = ProcessRequest::new("cliclick", args, timeout_ms.max(1));
    runner
        .run(&request)
        .map(|_| ())
        .map_err(|failure| map_failure("input.click", failure))
}

pub fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
}

#[cfg(test)]
mod tests {
    use super::button_name;
    use crate::cli::MouseButton;

    #[test]
    fn maps_button_name() {
        assert_eq!(button_name(MouseButton::Left), "left");
        assert_eq!(button_name(MouseButton::Right), "right");
        assert_eq!(button_name(MouseButton::Middle), "middle");
    }
}
