use anyhow::Result;
use clap::{Parser, Subcommand};
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use taut::{cache, config, depdb, discovery, output, runner, selection};

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

    /// Execution isolation mode
    #[arg(long, default_value = "process-per-test")]
    isolation: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List discovered tests without running them
    List {
        /// Path(s) to test files or directories
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,

        /// Filter tests by name substring
        #[arg(short = 'k', long)]
        filter: Option<String>,
    },
    /// Watch for changes and re-run affected tests
    Watch {
        /// Path(s) to test files or directories
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,

        /// Filter tests by name substring
        #[arg(short = 'k', long)]
        filter: Option<String>,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,

        /// Number of parallel jobs (default: CPU count)
        #[arg(short = 'j', long)]
        jobs: Option<usize>,

        /// Execution isolation mode
        #[arg(long, default_value = "process-per-test")]
        isolation: String,

        /// Disable dependency caching (run all tests)
        #[arg(long)]
        no_cache: bool,
    },
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

    match args.command {
        Some(Commands::List { paths, filter }) => list_tests(&paths, filter.as_deref()),
        Some(Commands::Watch {
            paths,
            filter,
            verbose,
            jobs,
            isolation,
            no_cache,
        }) => watch_tests(
            &paths,
            filter.as_deref(),
            verbose,
            jobs,
            &isolation,
            no_cache,
        ),
        Some(Commands::Cache { action }) => handle_cache_command(action),
        None => run_tests(args),
    }
}

fn list_tests(paths: &[PathBuf], filter: Option<&str>) -> Result<()> {
    let test_files = discovery::find_test_files(paths)?;

    if test_files.is_empty() {
        output::print_no_tests_found();
        return Ok(());
    }

    let all_tests = discovery::extract_tests(&test_files, filter)?;

    if all_tests.is_empty() {
        output::print_no_tests_found();
        return Ok(());
    }

    for test in &all_tests {
        println!("{}", test.id());
    }

    println!("\n{} tests", all_tests.len());
    Ok(())
}

fn watch_tests(
    paths: &[PathBuf],
    filter: Option<&str>,
    verbose: bool,
    jobs: Option<usize>,
    isolation: &str,
    no_cache: bool,
) -> Result<()> {
    // Load config from pyproject.toml
    let config = config::Config::load(&paths[0]);
    let jobs = jobs.or(config.max_workers);

    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                let _ = tx.send(event);
            }
        }
    })?;

    // Watch all provided paths
    for path in paths {
        let watch_path = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path.as_path()
        };
        watcher.watch(watch_path, RecursiveMode::Recursive)?;
    }

    println!("Watching for changes... (Ctrl+C to stop)\n");

    // Initial run
    run_tests_for_watch(paths, filter, verbose, jobs, isolation, no_cache);

    // Debounce: wait for events to settle
    loop {
        match rx.recv() {
            Ok(event) => {
                // Collect changed Python files
                let changed: Vec<_> = event
                    .paths
                    .iter()
                    .filter(|p| p.extension().map(|e| e == "py").unwrap_or(false))
                    .collect();

                if !changed.is_empty() {
                    // Drain any pending events (debounce)
                    std::thread::sleep(Duration::from_millis(100));
                    while rx.try_recv().is_ok() {}

                    // Show changed files
                    for path in &changed {
                        println!("changed: {}", path.display());
                    }
                    println!();

                    run_tests_for_watch(paths, filter, verbose, jobs, isolation, no_cache);
                }
            }
            Err(_) => break,
        }
    }

    Ok(())
}

fn run_tests_for_watch(
    paths: &[PathBuf],
    filter: Option<&str>,
    verbose: bool,
    jobs: Option<usize>,
    isolation: &str,
    no_cache: bool,
) {
    let test_files = match discovery::find_test_files(paths) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error discovering tests: {}", e);
            return;
        }
    };

    if test_files.is_empty() {
        output::print_no_tests_found();
        return;
    }

    let all_tests = match discovery::extract_tests(&test_files, filter) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error extracting tests: {}", e);
            return;
        }
    };

    if all_tests.is_empty() {
        output::print_no_tests_found();
        return;
    }

    let mut selector = selection::TestSelector::new();
    selector.index_files(paths);

    let (tests_to_run, skipped_tests) = if no_cache {
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

    let printer = output::ProgressPrinter::new(verbose);

    for result in &skipped_tests {
        printer.print_result(result);
    }

    let collect_coverage = !no_cache;
    let run_results = match runner::run_tests(
        &tests_to_run,
        true,
        jobs,
        collect_coverage,
        runner::IsolationMode::parse(isolation),
        |result| printer.print_result(result),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error running tests: {}", e);
            return;
        }
    };

    if !no_cache {
        for result in &run_results.results {
            selector.record_result(result);
        }
        selector.save();
    }

    let mut all_results = skipped_tests;
    all_results.extend(run_results.results);

    let combined = runner::TestResults {
        results: all_results,
        total_duration: run_results.total_duration,
    };

    let failed_tests = printer.get_failed_tests();
    output::print_summary(&combined, &failed_tests);
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
                println!(
                    "Total size: {:.1} KB ({} files)",
                    size_kb, cache_stats.file_count
                );
                println!();
                println!("Dependency database:");
                println!("  {} blocks tracked", depdb_stats.total_blocks);
                println!("  {} tests tracked", depdb_stats.total_tests);
                println!(
                    "  {} passed, {} failed",
                    depdb_stats.passed_tests, depdb_stats.failed_tests
                );
            }
        }
        CacheAction::Clear => {
            let (size_bytes, file_count) = cache::clear_cache()?;
            if file_count > 0 {
                let size_kb = size_bytes as f64 / 1024.0;
                println!("Cache cleared: {:.1} KB ({} files)", size_kb, file_count);
            } else {
                println!("Cache already empty.");
            }
        }
    }
    Ok(())
}

fn run_tests(args: Args) -> Result<()> {
    // Load config from pyproject.toml
    let config = config::Config::load(&args.paths[0]);

    // Resolve jobs: CLI flag > pyproject.toml > None (will use CPU count)
    let jobs = args.jobs.or(config.max_workers);

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

    // 4. Determine which tests to run (handle @skip markers first)
    let (mut tests_to_run, mut skipped_tests): (Vec<_>, Vec<_>) = if args.no_cache {
        // Run everything without caching, but still respect @skip markers
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

    // Handle @skip markers - move skipped tests to skipped_tests
    let (marker_skipped, remaining): (Vec<_>, Vec<_>) =
        tests_to_run.into_iter().partition(|item| item.is_skipped());

    tests_to_run = remaining;
    skipped_tests.extend(marker_skipped.into_iter().map(|item| {
        let reason = item
            .skip_reason()
            .unwrap_or_else(|| "marked with @skip".to_string());
        runner::skipped_result(&item, &reason)
    }));

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
        jobs,
        collect_coverage,
        runner::IsolationMode::parse(&args.isolation),
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
