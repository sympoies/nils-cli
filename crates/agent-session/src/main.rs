fn main() {
    const TOKEN_ENV: &str = "AGENT_SESSION_CONSOLE_ATTESTED_CREATE_TOKEN";
    let token = std::env::var(TOKEN_ENV).ok();
    // SAFETY: the CLI has only its main thread here. Keep the credential out
    // of all subprocess environments, including launch-profile probes.
    unsafe { std::env::remove_var(TOKEN_ENV) };
    std::process::exit(agent_session::run_with_serve_credential(
        std::env::args_os(),
        token,
    ));
}
