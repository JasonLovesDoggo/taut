use crate::runner::{TestResult, TestResults};
use colored::Colorize;
use std::io::{self, IsTerminal, Write};
use std::sync::Mutex;

#[derive(Default)]
struct ProgressState {
    started: bool,
    failed_tests: Vec<TestResult>,
}

pub struct ProgressPrinter {
    verbose: bool,
    quiet: bool,
    state: Mutex<ProgressState>,
}

impl ProgressPrinter {
    pub fn new(verbose: bool) -> Self {
        Self::with_options(verbose, false)
    }

    pub fn with_options(verbose: bool, quiet: bool) -> Self {
        Self {
            verbose,
            quiet,
            state: Mutex::new(ProgressState::default()),
        }
    }

    pub fn print_result(&self, result: &TestResult) {
        let mut state = self.state.lock().unwrap();
        if !result.passed && !result.skipped {
            state.failed_tests.push(result.clone());
        }
        if self.quiet {
            return;
        }
        let mut stdout = io::stdout().lock();
        if !state.started {
            if self.verbose {
                let _ = writeln!(stdout, "{}", "taut".bold());
            } else {
                let _ = write!(stdout, "{} ", "taut".bold());
            }
            state.started = true;
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

    pub fn get_failed_tests(&self) -> Vec<TestResult> {
        self.state.lock().unwrap().failed_tests.clone()
    }
}

fn print_failure(result: &TestResult) {
    println!("\n  {} {}", "F".red().bold(), result.item.id().bold());
    if let Some(error) = &result.error {
        if let Some(traceback) = error
            .traceback
            .as_deref()
            .filter(|traceback| !traceback.is_empty())
        {
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
}

/// Compatibility entry point for callers that already collected their failures.
pub fn print_summary(results: &TestResults, _failed_tests: &[TestResult]) {
    print_run_summary(results, results.results.len(), 0, false);
}

pub fn print_run_summary(results: &TestResults, collected: usize, unchanged: usize, quiet: bool) {
    if !quiet {
        println!();
    }
    for failure in results.results.iter().filter(|r| !r.passed && !r.skipped) {
        print_failure(failure);
    }
    let passed = results.passed_count();
    let failed = results.failed_count();
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
            traceback: None,
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
    }

    #[test]
    fn verbose_and_quiet_modes_retain_failures() {
        let mut result = skipped_result(&TestItem::default(), "");
        result.passed = false;
        result.skipped = false;
        for verbose in [false, true] {
            let printer = ProgressPrinter::with_options(verbose, true);
            printer.print_result(&result);
            assert_eq!(printer.get_failed_tests().len(), 1);
        }
    }
}
