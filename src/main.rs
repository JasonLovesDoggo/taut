use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use taut::{discovery, output, runner};

#[derive(Parser, Debug)]
#[command(name = "taut", version, about = "A fast Python test runner written in Rust")]
struct Args {
    /// Path(s) to test files or directories
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Filter tests by name substring
    #[arg(short = 'k', long)]
    filter: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Disable parallel execution
    #[arg(long)]
    no_parallel: bool,

    /// Number of parallel jobs (default: CPU count)
    #[arg(short = 'j', long)]
    jobs: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Discover test files
    let test_files = discovery::find_test_files(&args.paths)?;

    if test_files.is_empty() {
        output::print_no_tests_found();
        return Ok(());
    }

    // 2. Parse and extract test items
    let test_items = discovery::extract_tests(&test_files, args.filter.as_deref())?;

    if test_items.is_empty() {
        output::print_no_tests_found();
        return Ok(());
    }

    // 3. Run tests
    let results = runner::run_tests(&test_items, !args.no_parallel, args.jobs)?;

    // 4. Print summary
    output::print_summary(&results, args.verbose);

    // 5. Exit with appropriate code
    std::process::exit(if results.all_passed() { 0 } else { 1 });
}
