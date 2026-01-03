use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use taut::{cache, depdb, discovery, output, runner, selection};

#[derive(Parser, Debug)]
#[command(name = "taut", version, about = "Tests, without the overhead.")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

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

    /// Disable dependency caching (run all tests)
    #[arg(long)]
    no_cache: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Cache management commands
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand, Debug)]
enum CacheAction {
    /// Show cache statistics
    Info,
    /// Clear all cached data
    Clear,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(Commands::Cache { action }) = args.command {
        return handle_cache_command(action);
    }

    run_tests(args)
}

fn handle_cache_command(action: CacheAction) -> Result<()> {
    match action {
        CacheAction::Info => {
            let cache_stats = cache::get_cache_stats();
            let depdb_stats = depdb::DependencyDatabase::load().stats();

            println!("Cache location: {}", cache_stats.cache_dir.display());
            println!("Cache exists: {}", cache_stats.exists);

            if cache_stats.exists {
                let size_kb = cache_stats.size_bytes as f64 / 1024.0;
                println!("Total size: {:.1} KB ({} files)", size_kb, cache_stats.file_count);
                println!();
                println!("Dependency database:");
                println!("  {} blocks tracked", depdb_stats.total_blocks);
                println!("  {} tests tracked", depdb_stats.total_tests);
                println!("  {} passed, {} failed", depdb_stats.passed_tests, depdb_stats.failed_tests);
            }
        }
        CacheAction::Clear => {
            cache::clear_cache()?;
            println!("Cache cleared.");
        }
    }
    Ok(())
}

fn run_tests(args: Args) -> Result<()> {
    // 1. Discover test files
    let test_files = discovery::find_test_files(&args.paths)?;

    if test_files.is_empty() {
        output::print_no_tests_found();
        return Ok(());
    }

    // 2. Parse and extract test items
    let all_tests = discovery::extract_tests(&test_files, args.filter.as_deref())?;

    if all_tests.is_empty() {
        output::print_no_tests_found();
        return Ok(());
    }

    // 3. Set up test selector for dependency tracking
    let mut selector = selection::TestSelector::new();

    // Index all Python files in the search paths for coverage mapping
    selector.index_files(&args.paths);

    // 4. Determine which tests to run
    let (tests_to_run, skipped_tests) = if args.no_cache {
        // Run everything without caching
        (all_tests.clone(), Vec::new())
    } else {
        let selection = selector.select_tests(&all_tests);
        let to_run: Vec<_> = selection.to_run.into_iter().map(|(item, _)| item).collect();
        let skipped: Vec<_> = selection
            .to_skip
            .into_iter()
            .map(|(item, reason)| runner::skipped_result(&item, &reason))
            .collect();
        (to_run, skipped)
    };

    // 5. Run tests with streaming output
    let printer = output::ProgressPrinter::new(args.verbose);

    // Print skipped tests first
    for result in &skipped_tests {
        printer.print_result(result);
    }

    // Run actual tests with coverage collection (when caching enabled)
    let collect_coverage = !args.no_cache;
    let run_results = runner::run_tests(
        &tests_to_run,
        !args.no_parallel,
        args.jobs,
        collect_coverage,
        |result| printer.print_result(result),
    )?;

    // 6. Record coverage for dependency tracking
    if !args.no_cache {
        for result in &run_results.results {
            selector.record_result(result);
        }
        selector.save();
    }

    // 7. Combine results
    let mut all_results = skipped_tests;
    all_results.extend(run_results.results);

    let combined = runner::TestResults {
        results: all_results,
        total_duration: run_results.total_duration,
    };

    // 8. Print summary
    let failed_tests = printer.get_failed_tests();
    output::print_summary(&combined, &failed_tests);

    // 9. Exit with appropriate code
    std::process::exit(if combined.all_passed() { 0 } else { 1 });
}
