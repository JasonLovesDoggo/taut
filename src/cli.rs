//! A single execution path for one-shot and watch runs.

use crate::{cache, config, depdb, discovery, output, runner, selection};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use notify::{RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(
    name = "taut",
    version,
    about = "Tests, without the overhead.",
    after_help = "Exit codes: 0 passed, 1 test failures, 2 usage or configuration error, 5 no tests collected."
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,
    /// Test files or directories [default: .]
    pub paths: Vec<PathBuf>,
    #[command(flatten)]
    pub options: Options,
    /// Generate markdown documentation for CLI
    #[arg(long, hide = true)]
    pub markdown_help: bool,
}

#[derive(clap::Args, Debug, Default)]
pub struct Options {
    /// Filter tests by name expression
    #[arg(short = 'k', long, global = true)]
    pub filter: Option<String>,
    /// Print individual test names and timings
    #[arg(short, long, global = true, conflicts_with_all = ["quiet", "json"])]
    pub verbose: bool,
    /// Print only failures and the final summary
    #[arg(short, long, global = true, conflicts_with = "json")]
    pub quiet: bool,
    /// Emit one machine-readable JSON document
    #[arg(long, global = true)]
    pub json: bool,
    /// Run tests sequentially
    #[arg(long, global = true)]
    pub no_parallel: bool,
    /// Number of worker processes [default: CPU count]
    #[arg(short = 'j', long, global = true, value_parser = positive_usize)]
    pub jobs: Option<usize>,
    /// Run only tests affected by tracked dependency changes (enables tracing)
    #[arg(long, global = true, conflicts_with = "no_cache")]
    pub changed: bool,
    /// Run all tests without dependency tracing (the default)
    #[arg(long, global = true)]
    pub no_cache: bool,
    /// Worker lifetime [default: process-per-run]
    #[arg(long, global = true, value_enum)]
    pub isolation: Option<Isolation>,
    /// Python executable or path [default: active virtual environment, .venv, python3]
    #[arg(long, global = true, value_name = "EXECUTABLE")]
    pub python: Option<PathBuf>,
    /// Async tests sharing each worker's event loop [default: 1]
    #[arg(long, global = true, value_parser = positive_usize)]
    pub async_concurrency: Option<usize>,
    /// Per-test timeout in seconds
    #[arg(long, global = true, value_parser = timeout_seconds, value_name = "SECONDS")]
    pub timeout: Option<f64>,
    /// Stop scheduling tests after the first failure
    #[arg(short = 'x', long, global = true)]
    pub fail_fast: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum Isolation {
    ProcessPerRun,
    ProcessPerTest,
}

impl From<Isolation> for runner::IsolationMode {
    fn from(value: Isolation) -> Self {
        match value {
            Isolation::ProcessPerRun => Self::ProcessPerRun,
            Isolation::ProcessPerTest => Self::ProcessPerTest,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List discovered tests without importing or executing Python
    List {
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
    },
    /// Re-run tests when Python or project configuration files change
    Watch {
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
    },
    /// Manage dependency selection data
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum CacheAction {
    /// Show dependency selection cache statistics
    Info,
    /// Remove cached dependency selection data
    Clear,
}

fn positive_usize(value: &str) -> std::result::Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "must be an integer greater than zero".to_owned())
}

fn timeout_seconds(value: &str) -> std::result::Result<f64, String> {
    let seconds = value
        .parse::<f64>()
        .map_err(|_| "must be a number of seconds".to_owned())?;
    config::parse_timeout(seconds).map_err(|e| e.to_string())?;
    Ok(seconds)
}

pub fn run() -> i32 {
    run_with_args(std::env::args().collect())
}

pub fn run_with_args(args: Vec<String>) -> i32 {
    match Args::try_parse_from(args) {
        Ok(args) => run_with_parsed_args(args),
        Err(error) => {
            let code = error.exit_code();
            let _ = error.print();
            code
        }
    }
}

fn run_with_parsed_args(args: Args) -> i32 {
    if args.markdown_help {
        print!("{}", generate_markdown_help());
        return 0;
    }
    let result = match args.command {
        Some(Commands::List { paths }) => list_tests(&paths, &args.options),
        Some(Commands::Watch { paths }) => watch_tests(&paths, &args.options),
        Some(Commands::Cache { action }) => handle_cache_command(action, args.options.json),
        None => {
            let paths = if args.paths.is_empty() {
                vec![PathBuf::from(".")]
            } else {
                args.paths
            };
            execute(&paths, &args.options)
        }
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            print_error(&error, args.options.json);
            2
        }
    }
}

fn print_error(error: &anyhow::Error, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({"schema_version": 1, "error": format!("{error:#}")})
        );
    } else {
        eprintln!("error: {error:#}");
    }
}

pub fn generate_markdown_help() -> String {
    clap_markdown::help_markdown::<Args>()
}

fn selection_path(path: &Path) -> PathBuf {
    path.to_str()
        .and_then(|value| value.split_once("::"))
        .map(|(file, _)| PathBuf::from(file))
        .unwrap_or_else(|| path.to_path_buf())
}

fn validate_paths(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        if !selection_path(path).exists() {
            bail!(
                "test path does not exist: {}",
                selection_path(path).display()
            );
        }
    }
    Ok(())
}

fn collect(paths: &[PathBuf], options: &Options) -> Result<Vec<discovery::TestItem>> {
    validate_paths(paths)?;
    discovery::collect_tests(paths, options.filter.as_deref())
}

fn list_tests(paths: &[PathBuf], options: &Options) -> Result<i32> {
    validate_paths(paths)?;
    config::Config::load(&selection_path(&paths[0]))?;
    let tests = collect(paths, options)?;
    if options.json {
        println!(
            "{}",
            serde_json::json!({"schema_version": 1, "tests": tests.iter().map(|t| t.id()).collect::<Vec<_>>(), "collected": tests.len()})
        );
    } else if tests.is_empty() {
        output::print_no_tests_found();
    } else {
        for test in &tests {
            println!("{}", test.id());
        }
        if !options.quiet {
            println!("\n{} tests", tests.len());
        }
    }
    Ok(if tests.is_empty() { 5 } else { 0 })
}

fn runner_options(options: &Options, config: config::Config) -> Result<runner::RunOptions> {
    let isolation = options.isolation.map(Into::into).unwrap_or_else(|| {
        runner::IsolationMode::parse(config.isolation.as_deref().unwrap_or("process-per-run"))
    });
    let async_concurrency = options
        .async_concurrency
        .or(config.async_concurrency)
        .unwrap_or(1);
    if options.no_parallel && async_concurrency > 1 {
        bail!("--no-parallel requires --async-concurrency 1");
    }
    if matches!(isolation, runner::IsolationMode::ProcessPerTest) && async_concurrency > 1 {
        bail!(
            "--async-concurrency greater than 1 requires --isolation process-per-run; isolated tests cannot share an event loop"
        );
    }
    let python = options.python.clone().or(config.python);
    if python.as_ref().is_some_and(|p| p.as_os_str().is_empty()) {
        bail!("--python must name an interpreter or executable path");
    }
    Ok(runner::RunOptions {
        parallel: !options.no_parallel,
        jobs: options.jobs.or(config.max_workers),
        collect_coverage: options.changed,
        isolation,
        python,
        async_concurrency,
        timeout: options
            .timeout
            .or(config.timeout)
            .map(config::parse_timeout)
            .transpose()?,
        fail_fast: options.fail_fast || config.fail_fast,
    })
}

fn execution_context(options: &runner::RunOptions) -> String {
    let python = crate::worker_pool::resolve_python(options.python.as_deref());
    let canonical = python.canonicalize().ok();
    let metadata = std::fs::metadata(&python)
        .ok()
        .map(|metadata| (metadata.len(), metadata.modified().ok()));
    format!(
        "python={python:?};canonical={canonical:?};metadata={metadata:?};isolation={:?};parallel={};jobs={:?};async={};timeout={:?};fail_fast={}",
        options.isolation,
        options.parallel,
        options.jobs,
        options.async_concurrency,
        options.timeout,
        options.fail_fast
    )
}

fn execute(paths: &[PathBuf], options: &Options) -> Result<i32> {
    let started = Instant::now();
    validate_paths(paths)?;
    let runtime = runner_options(options, config::Config::load(&selection_path(&paths[0]))?)?;
    let all_tests = collect(paths, options)?;
    if all_tests.is_empty() {
        if options.json {
            output::print_json(
                &runner::TestResults {
                    results: Vec::new(),
                    total_duration: started.elapsed(),
                },
                0,
                &[],
            );
        } else {
            output::print_no_tests_found();
        }
        return Ok(5);
    }
    let collected = all_tests.len();
    // Normal runs never instantiate the selector or parse unrelated source files.
    let mut selector = options.changed.then(selection::TestSelector::new);
    if let Some(selector) = &mut selector {
        selector.set_execution_context(&execution_context(&runtime));
    }
    let mut selected_out = Vec::new();
    let mut skipped = Vec::new();
    let mut runnable = Vec::new();
    for test in all_tests {
        if test.is_skipped() {
            let reason = test
                .skip_reason()
                .unwrap_or_else(|| "marked with @skip".to_owned());
            skipped.push(runner::skipped_result(&test, &reason));
        } else {
            runnable.push(test);
        }
    }
    if let Some(selector) = &mut selector {
        selector.index_files(
            &paths
                .iter()
                .map(|path| selection_path(path))
                .collect::<Vec<_>>(),
        );
        let selection = selector.select_tests(&runnable);
        runnable = selection.to_run.into_iter().map(|(test, _)| test).collect();
        selected_out = selection
            .to_skip
            .into_iter()
            .map(|(test, _)| test.id())
            .collect();
    }
    let printer =
        output::ProgressPrinter::with_options(options.verbose, options.quiet || options.json);
    for result in &skipped {
        printer.print_result(result);
    }
    let run =
        runner::run_tests_with_options(&runnable, &runtime, |result| printer.print_result(result))?;
    if let Some(selector) = &mut selector {
        for result in &run.results {
            selector.record_result(result);
        }
        selector.save();
    }
    skipped.extend(run.results);
    let results = runner::TestResults {
        results: skipped,
        total_duration: started.elapsed(),
    };
    if options.json {
        output::print_json(&results, collected, &selected_out);
    } else {
        output::print_run_summary(&results, collected, selected_out.len(), options.quiet);
    }
    Ok(if results.all_passed() { 0 } else { 1 })
}

fn watch_tests(paths: &[PathBuf], options: &Options) -> Result<i32> {
    validate_paths(paths)?;
    // Validate configuration before starting a long-lived watch session.
    runner_options(options, config::Config::load(&selection_path(&paths[0]))?)?;
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let _ = tx.send(event);
    })?;
    let mut roots = BTreeSet::new();
    for path in paths {
        let absolute = selection_path(path).canonicalize()?;
        let directory = if absolute.is_file() {
            absolute.parent().unwrap().to_path_buf()
        } else {
            absolute
        };
        // Source files and pyproject.toml may live above a selected tests directory.
        let root = directory
            .ancestors()
            .find(|ancestor| ancestor.join("pyproject.toml").is_file())
            .unwrap_or(&directory)
            .to_path_buf();
        roots.insert(root);
    }
    let watch_roots: Vec<_> = roots
        .iter()
        .filter(|path| {
            !roots
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .collect();
    for path in watch_roots {
        watcher.watch(path, RecursiveMode::Recursive)?;
    }
    if !options.json && !options.quiet {
        eprintln!("Watching for changes... (Ctrl+C to stop)");
    }
    if let Err(error) = execute(paths, options) {
        print_error(&error, options.json);
    }
    loop {
        let first = rx.recv().context("filesystem watcher disconnected")??;
        let mut changed = BTreeSet::new();
        record_changes(first, &mut changed);
        // Reset the short debounce window for every arriving event.
        while let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
            record_changes(event?, &mut changed);
        }
        if changed.is_empty() {
            continue;
        }
        if !options.json && !options.quiet {
            for path in changed {
                eprintln!("changed: {}", path.display());
            }
        }
        if let Err(error) = execute(paths, options) {
            print_error(&error, options.json);
        }
    }
}

fn record_changes(event: notify::Event, paths: &mut BTreeSet<PathBuf>) {
    if !(event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove()) {
        return;
    }
    paths.extend(event.paths.into_iter().filter(|path| watch_relevant(path)));
}

fn watch_relevant(path: &Path) -> bool {
    !path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(
                ".git"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".taut"
                    | ".tox"
                    | ".nox"
                    | "node_modules"
                    | "target"
                    | "build"
                    | "dist"
            )
        )
    }) && (path.extension().is_some_and(|ext| ext == "py")
        || path
            .file_name()
            .is_some_and(|name| name == "pyproject.toml"))
}

fn handle_cache_command(action: CacheAction, json: bool) -> Result<i32> {
    match action {
        CacheAction::Info => {
            let cache = cache::get_cache_stats();
            let dependencies = depdb::DependencyDatabase::load().stats();
            if json {
                println!(
                    "{}",
                    serde_json::json!({"schema_version": 1, "cache": {"path": cache.cache_dir, "exists": cache.exists, "size_bytes": cache.size_bytes, "files": cache.file_count, "blocks": dependencies.total_blocks, "tests": dependencies.total_tests}})
                );
                return Ok(0);
            }
            println!("Cache location: {}", cache.cache_dir.display());
            println!("Cache exists: {}", cache.exists);
            println!(
                "Total size: {:.1} KB ({} files)",
                cache.size_bytes as f64 / 1024.0,
                cache.file_count
            );
            println!(
                "{} blocks tracked, {} tests tracked",
                dependencies.total_blocks, dependencies.total_tests
            );
        }
        CacheAction::Clear => {
            let (bytes, files) = cache::clear_cache()?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"schema_version": 1, "cleared_bytes": bytes, "cleared_files": files})
                );
            } else if files == 0 {
                println!("Cache already empty.");
            } else {
                println!(
                    "Cache cleared: {:.1} KB ({} files)",
                    bytes as f64 / 1024.0,
                    files
                );
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_options_are_accepted_after_subcommands() {
        let args = Args::try_parse_from([
            "taut",
            "watch",
            "tests",
            "--python",
            "python3.13",
            "--async-concurrency",
            "8",
            "-x",
            "-j",
            "2",
        ])
        .unwrap();
        assert_eq!(args.options.jobs, Some(2));
        assert_eq!(args.options.async_concurrency, Some(8));
        assert!(args.options.fail_fast);
    }

    #[test]
    fn invalid_flags_are_usage_errors() {
        for args in [
            vec!["taut", "-j", "0"],
            vec!["taut", "--async-concurrency", "0"],
            vec!["taut", "--isolation", "typo"],
            vec!["taut", "--timeout", "nan"],
            vec!["taut", "--timeout", "0"],
            vec!["taut", "--changed", "--no-cache"],
        ] {
            assert_eq!(Args::try_parse_from(args).unwrap_err().exit_code(), 2);
        }
    }

    #[test]
    fn help_and_version_are_successful() {
        for flag in ["--help", "--version"] {
            assert_eq!(
                Args::try_parse_from(["taut", flag])
                    .unwrap_err()
                    .exit_code(),
                0
            );
        }
    }

    #[test]
    fn watch_ignores_generated_python_files() {
        assert!(watch_relevant(Path::new("src/example.py")));
        assert!(watch_relevant(Path::new("pyproject.toml")));
        assert!(!watch_relevant(Path::new(".venv/lib/test_fake.py")));
        assert!(!watch_relevant(Path::new("__pycache__/test.py")));
    }
}
