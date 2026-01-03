pub mod blocks;
pub mod cache;
pub mod cli;
pub mod config;
pub mod depdb;
pub mod discovery;
pub mod filter;
pub mod markers;
pub mod output;
pub mod runner;
pub mod selection;
pub mod worker_pool;

#[cfg(feature = "extension-module")]
use pyo3::prelude::*;

/// CLI entry point for `taut` command.
/// Called from Python via console_scripts entrypoint.
#[cfg(feature = "extension-module")]
#[pyfunction]
fn main() {
    let code = cli::run();
    std::process::exit(code);
}

/// PyO3 module definition
#[cfg(feature = "extension-module")]
#[pymodule]
fn _taut(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(main, m)?)?;
    Ok(())
}
