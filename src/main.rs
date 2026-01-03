use std::process::ExitCode;

fn main() -> ExitCode {
    let code = taut::cli::run();
    ExitCode::from(code as u8)
}
