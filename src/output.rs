use crate::runner::{TestResult, TestResults};
use colored::Colorize;
use std::io::{self, Write};
use std::sync::Mutex;

pub struct ProgressPrinter {
    verbose: bool,
    printed_header: Mutex<bool>,
    failed_tests: Mutex<Vec<TestResult>>,
}

impl ProgressPrinter {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose,
            printed_header: Mutex::new(false),
            failed_tests: Mutex::new(Vec::new()),
        }
    }

    fn print_header(&self) {
        let mut printed = self.printed_header.lock().unwrap();
        if !*printed {
            print!("{} ", "taut".bold());
            let _ = io::stdout().flush();
            *printed = true;
        }
    }

    pub fn print_result(&self, result: &TestResult) {
        self.print_header();

        if self.verbose {
            self.print_verbose(result);
        } else {
            self.print_compact(result);
        }
    }

    fn print_compact(&self, result: &TestResult) {
        let symbol = if result.skipped {
            "s".cyan()
        } else if result.passed {
            ".".green()
        } else {
            "F".red()
        };

        print!("{}", symbol);
        let _ = io::stdout().flush();

        // Store failed tests for later
        if !result.passed && !result.skipped
            && let Ok(mut failed) = self.failed_tests.lock() {
                failed.push(result.clone());
            }
    }

    fn print_verbose(&self, result: &TestResult) {
        // Print newline before first verbose result
        {
            let printed = self.printed_header.lock().unwrap();
            if *printed {
                static FIRST_VERBOSE: std::sync::Once = std::sync::Once::new();
                FIRST_VERBOSE.call_once(|| println!());
            }
        }

        let symbol = if result.skipped {
            "○".cyan()
        } else if result.passed {
            "✓".green()
        } else {
            "✗".red()
        };

        let duration_str = if result.skipped {
            result
                .skip_reason
                .as_deref()
                .unwrap_or("skipped")
                .to_string()
        } else {
            format!("{}ms", result.duration.as_millis())
        };

        let name = if let Some(ref class) = result.item.class {
            format!("{}::{}", class, result.item.function)
        } else {
            result.item.function.clone()
        };

        let file = result.item.file.display();
        println!(
            "  {} {}::{} ({})",
            symbol,
            file.to_string().dimmed(),
            name,
            duration_str
        );

        if !result.passed && !result.skipped
            && let Some(ref error) = result.error {
                println!("    {}", error.message.red());
                if let Some(ref tb) = error.traceback {
                    for line in tb.lines().take(10) {
                        println!("    {}", line.dimmed());
                    }
                }
            }

        let _ = io::stdout().flush();
    }

    pub fn get_failed_tests(&self) -> Vec<TestResult> {
        self.failed_tests.lock().unwrap().clone()
    }
}

pub fn print_summary(results: &TestResults, failed_tests: &[TestResult]) {
    println!();

    // Print failures
    if !failed_tests.is_empty() {
        println!();
        println!("{}", "Failures:".red().bold());
        for result in failed_tests {
            let name = if let Some(ref class) = result.item.class {
                format!("{}::{}", class, result.item.function)
            } else {
                result.item.function.clone()
            };
            println!();
            println!(
                "  {} {}::{}",
                "✗".red(),
                result.item.file.display().to_string().dimmed(),
                name
            );
            if let Some(ref error) = result.error {
                println!("    {}", error.message.red());
                println!("    {}:{}", result.item.file.display(), result.item.line);
            }
        }
        println!();
    }

    let passed = results.passed_count();
    let failed = results.failed_count();
    let skipped = results.skipped_count();
    let duration = results.total_duration.as_secs_f64();

    let mut parts = Vec::new();
    parts.push(format!("{} passed", passed));
    if failed > 0 {
        parts.push(format!("{} failed", failed));
    }
    if skipped > 0 {
        parts.push(format!("{} skipped", skipped));
    }
    parts.push(format!("in {:.2}s", duration));

    let summary = parts.join(", ");
    if failed == 0 {
        println!("{}", summary.green());
    } else {
        println!("{}", summary.red());
    }
}

pub fn print_no_tests_found() {
    println!("{}", "taut".bold());
    println!("{}", "No tests found.".yellow());
}
