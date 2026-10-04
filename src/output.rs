use crate::runner::{IsolationMode, RunOptions, TestResult, TestResults};
use colored::Colorize;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct ProgressPrinter {
    verbose: bool,
    quiet: bool,
    started: Mutex<bool>,
}

impl ProgressPrinter {
    pub fn new(verbose: bool) -> Self {
        Self::with_options(verbose, false)
    }

    pub fn with_options(verbose: bool, quiet: bool) -> Self {
        Self {
            verbose,
            quiet,
            started: Mutex::new(false),
        }
    }

    pub fn print_result(&self, result: &TestResult) {
        if self.quiet {
            return;
        }
        let mut started = self.started.lock().unwrap();
        let mut stdout = io::stdout().lock();
        if !*started {
            if self.verbose {
                let _ = writeln!(stdout, "{}", "taut".bold());
            } else {
                let _ = write!(stdout, "{} ", "taut".bold());
            }
            *started = true;
        }
        let symbol = if result.skipped {
            "s".cyan()
        } else if result.passed {
            ".".green()
        } else {
            "F".red()
        };
        if self.verbose {
            let detail = if result.skipped {
                result
                    .skip_reason
                    .clone()
                    .unwrap_or_else(|| "skipped".to_owned())
            } else {
                format!("{:.2}ms", result.duration.as_secs_f64() * 1000.0)
            };
            let _ = writeln!(stdout, "  {symbol} {} ({detail})", result.item.id());
        } else {
            let _ = write!(stdout, "{symbol}");
        }
        // Avoid a syscall per result when output is redirected (CI/benchmarks).
        if stdout.is_terminal() {
            let _ = stdout.flush();
        }
    }
}

#[derive(Default)]
pub struct SummaryOptions<'a> {
    pub quiet: bool,
    pub verbose: bool,
    pub runtime: Option<&'a RunOptions>,
    /// Stable interpreter selection; launcher-managed environments are reselected.
    pub rerun_python: Option<&'a Path>,
}

#[derive(Clone, Copy)]
enum Shell {
    Posix,
    PowerShell,
}

impl Shell {
    fn native() -> Self {
        if cfg!(windows) {
            Self::PowerShell
        } else {
            Self::Posix
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Posix => "POSIX",
            Self::PowerShell => "PowerShell 7.3+",
        }
    }

    fn quote(self, value: &str) -> String {
        if !value.is_empty()
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_./:=-".contains(&c))
        {
            return value.to_owned();
        }
        let escaped = match self {
            Self::Posix => value.replace('\'', "'\"'\"'"),
            // PowerShell recognizes typographic apostrophes as delimiters too.
            Self::PowerShell => value.chars().fold(String::new(), |mut escaped, c| {
                if matches!(c, '\'' | '\u{2018}'..='\u{201b}') {
                    escaped.push(c);
                }
                escaped.push(c);
                escaped
            }),
        };
        format!("'{escaped}'")
    }

    fn command(self, arguments: &[String]) -> String {
        let mut command = match self {
            Self::Posix => String::new(),
            Self::PowerShell => "& ".to_owned(),
        };
        command.push_str(
            &arguments
                .iter()
                .map(|argument| self.quote(argument))
                .collect::<Vec<_>>()
                .join(" "),
        );
        command
    }
}

fn rerun_arguments(result: &TestResult, options: &SummaryOptions<'_>) -> Vec<String> {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("taut"));
    let executable = std::env::current_dir()
        .ok()
        .and_then(|cwd| executable.strip_prefix(cwd).ok().map(PathBuf::from))
        .map(|relative| PathBuf::from(".").join(relative))
        .unwrap_or(executable);
    let mut arguments = vec![
        executable.to_string_lossy().into_owned(),
        "--no-config".to_owned(),
    ];
    if let Some(python) = options.rerun_python {
        arguments.extend(["--python".to_owned(), python.to_string_lossy().into_owned()]);
    }
    if let Some(runtime) = options.runtime {
        if !runtime.parallel {
            arguments.push("--no-parallel".to_owned());
        }
        if runtime.isolation != IsolationMode::ProcessPerRun {
            arguments.extend(["--isolation".to_owned(), "process-per-test".to_owned()]);
        }
        if runtime.async_concurrency != 1 {
            arguments.extend([
                "--async-concurrency".to_owned(),
                runtime.async_concurrency.to_string(),
            ]);
        }
        if let Some(timeout) = runtime.timeout {
            arguments.extend(["--timeout".to_owned(), timeout.as_secs_f64().to_string()]);
        }
    }
    arguments.extend(["--".to_owned(), result.item.id()]);
    arguments
}

fn print_failure(result: &TestResult, options: &SummaryOptions<'_>) {
    println!("\n  {} {}", "F".red().bold(), result.item.id().bold());
    if let Some(error) = &result.error {
        let traceback = if options.verbose {
            error.traceback.as_deref()
        } else {
            error
                .focused_traceback
                .as_deref()
                .or(error.traceback.as_deref())
        };
        if let Some(traceback) = traceback.filter(|traceback| !traceback.is_empty()) {
            for line in traceback.lines() {
                println!("    {line}");
            }
        } else {
            println!("    {}", error.message.red());
            println!("    {}:{}", result.item.file.display(), result.item.line);
        }
    }
    for (label, value) in [
        ("Captured stdout", &result.stdout),
        ("Captured stderr", &result.stderr),
    ] {
        if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
            println!("    --- {label} ---");
            for line in value.lines() {
                println!("    {line}");
            }
        }
    }
    let shell = Shell::native();
    println!(
        "    Rerun ({}): {}",
        shell.name(),
        shell.command(&rerun_arguments(result, options))
    );
}

/// Compatibility entry point for callers that already collected their failures.
pub fn print_summary(results: &TestResults, _failed_tests: &[TestResult]) {
    print_run_summary(results, results.results.len(), 0, false);
}

pub fn print_run_summary(results: &TestResults, collected: usize, unchanged: usize, quiet: bool) {
    print_run_summary_with_options(
        results,
        collected,
        unchanged,
        &SummaryOptions {
            quiet,
            ..SummaryOptions::default()
        },
    );
}

pub fn print_run_summary_with_options(
    results: &TestResults,
    collected: usize,
    unchanged: usize,
    options: &SummaryOptions<'_>,
) {
    if !options.quiet {
        println!();
    }
    for failure in results.results.iter().filter(|r| !r.passed && !r.skipped) {
        print_failure(failure, options);
    }
    let passed = results.passed_count();
    let failed = results.failed_count();
    if failed > 0 && options.runtime.is_some() && options.rerun_python.is_none() {
        println!("Keep the same uv run prefix or activated environment when rerunning.");
    }
    let skipped = results.skipped_count();
    let not_run = collected.saturating_sub(results.results.len() + unchanged);
    let mut parts = vec![format!("{passed} passed")];
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    if unchanged > 0 {
        parts.push(format!("{unchanged} unchanged"));
    }
    if not_run > 0 {
        parts.push(format!("{not_run} not run"));
    }
    parts.push(format!("in {:.2}s", results.total_duration.as_secs_f64()));
    let summary = parts.join(", ");
    if failed == 0 {
        println!("{}", summary.green());
    } else {
        println!("{}", summary.red());
    }
}

pub fn json_report(
    results: &TestResults,
    collected: usize,
    selected_out: &[String],
) -> serde_json::Value {
    let tests = results.results.iter().map(|result| serde_json::json!({
        "id": result.item.id(),
        "file": result.item.file,
        "line": result.item.line,
        "status": if result.skipped { "skipped" } else if result.passed { "passed" } else { "failed" },
        "duration_seconds": result.duration.as_secs_f64(),
        "skip_reason": result.skip_reason,
        "error": result.error,
        "stdout": result.stdout,
        "stderr": result.stderr,
    })).collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": 1,
        "summary": {
            "collected": collected,
            "executed": results.passed_count() + results.failed_count(),
            "passed": results.passed_count(),
            "failed": results.failed_count(),
            "skipped": results.skipped_count(),
            "unchanged": selected_out.len(),
            "not_run": collected.saturating_sub(results.results.len() + selected_out.len()),
            "duration_seconds": results.total_duration.as_secs_f64(),
        },
        "tests": tests,
        "selected_out": selected_out,
    })
}

pub fn print_json(results: &TestResults, collected: usize, selected_out: &[String]) {
    println!("{}", json_report(results, collected, selected_out));
}

pub fn print_no_tests_found() {
    println!("{}", "No tests found.".yellow());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::TestItem;
    use crate::runner::{TestError, skipped_result};
    use std::time::Duration;

    #[test]
    fn report_keeps_executed_skipped_unchanged_and_not_run_separate() {
        let skipped = skipped_result(&TestItem::default(), "intentional skip");
        let mut failed = skipped.clone();
        failed.skipped = false;
        failed.passed = false;
        failed.error = Some(TestError {
            message: "failure".into(),
            traceback: Some("raw worker frames".into()),
            focused_traceback: Some("focused user frames".into()),
        });
        let results = TestResults {
            results: vec![skipped, failed],
            total_duration: Duration::from_millis(5),
        };
        let report = json_report(&results, 4, &["unchanged_test".into()]);
        assert_eq!(report["summary"]["executed"], 1);
        assert_eq!(report["summary"]["skipped"], 1);
        assert_eq!(report["summary"]["unchanged"], 1);
        assert_eq!(report["summary"]["not_run"], 1);
        assert_eq!(report["tests"][1]["error"]["message"], "failure");
        assert_eq!(
            report["tests"][1]["error"]["traceback"],
            "raw worker frames"
        );
        assert!(
            report["tests"][1]["error"]
                .get("focused_traceback")
                .is_none()
        );
    }

    #[test]
    fn rerun_preserves_execution_options_without_selection_or_environment_values() {
        let result = skipped_result(
            &TestItem {
                file: "-test's file.py".into(),
                function: "test_value".into(),
                ..TestItem::default()
            },
            "",
        );
        let runtime = RunOptions {
            python: Some(".venv with spaces/bin/python".into()),
            isolation: IsolationMode::ProcessPerTest,
            parallel: false,
            timeout: Some(Duration::from_millis(1250)),
            collect_coverage: true,
            fail_fast: true,
            jobs: Some(8),
            ..RunOptions::default()
        };
        let arguments = rerun_arguments(
            &result,
            &SummaryOptions {
                runtime: Some(&runtime),
                rerun_python: runtime.python.as_deref(),
                ..SummaryOptions::default()
            },
        );
        assert_eq!(
            &arguments[1..],
            [
                "--no-config",
                "--python",
                ".venv with spaces/bin/python",
                "--no-parallel",
                "--isolation",
                "process-per-test",
                "--timeout",
                "1.25",
                "--",
                "-test's file.py::test_value"
            ]
        );
        let concurrent = RunOptions {
            async_concurrency: 4,
            python: Some("python3".into()),
            ..RunOptions::default()
        };
        let arguments = rerun_arguments(
            &result,
            &SummaryOptions {
                runtime: Some(&concurrent),
                rerun_python: concurrent.python.as_deref(),
                ..SummaryOptions::default()
            },
        );
        assert_eq!(
            &arguments[1..],
            [
                "--no-config",
                "--python",
                "python3",
                "--async-concurrency",
                "4",
                "--",
                "-test's file.py::test_value"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn posix_commands_round_trip_literal_arguments() {
        let values = [
            "test file.py::test_value[it's \"quoted\"]",
            "$(printf unsafe); `printf unsafe` & * ? [a]",
            "-test_file.py::test_value[界 ‘ ’ ‚ ‛]",
            "line\nbreak\ttab\\backslash",
            "",
        ];
        let mut arguments = vec!["printf".to_owned(), "%s\\0".to_owned()];
        arguments.extend(values.iter().map(|value| (*value).to_owned()));
        let command = Shell::Posix.command(&arguments);
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut expected = Vec::new();
        for value in values {
            expected.extend_from_slice(value.as_bytes());
            expected.push(0);
        }
        assert_eq!(output.stdout, expected);
    }

    #[test]
    fn powershell_quotes_literal_arguments_and_invokes_quoted_executable() {
        let command = Shell::PowerShell.command(&[
            "C:\\Program Files\\taut.exe".into(),
            "--".into(),
            "test file.py::test_value[it's \"quoted\" $(value) ` ; &]".into(),
        ]);
        assert_eq!(
            command,
            "& 'C:\\Program Files\\taut.exe' -- 'test file.py::test_value[it''s \"quoted\" $(value) ` ; &]'"
        );
        assert_eq!(Shell::PowerShell.quote("界 ‘ ’ ‚ ‛"), "'界 ‘‘ ’’ ‚‚ ‛‛'");
        assert_eq!(Shell::PowerShell.quote(""), "''");
    }
}
