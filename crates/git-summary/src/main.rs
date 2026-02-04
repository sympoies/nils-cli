mod app;
mod cli;
mod dates;
mod git;
mod summary;

fn main() {
    std::process::exit(app::run());
}
