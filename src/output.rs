use crate::runner::{TestResult, TestResults};
use colored::Colorize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn print_result(result: &TestResult, verbose: bool) {
    let symbol = if result.passed {
        "✓".green()
    } else {
        "✗".red()
    };

    let duration_ms = result.duration.as_millis();
    let name = if let Some(ref class) = result.item.class {
        format!("{}::{}", class, result.item.function)
    } else {
        result.item.function.clone()
    };

    println!("  {} {} ({}ms)", symbol, name, duration_ms);

    if !result.passed {
        if let Some(ref error) = result.error {
            println!("    {}", error.message.red());
            println!("    {}:{}", result.item.file.display(), result.item.line);
            if verbose {
                if let Some(ref tb) = error.traceback {
                    for line in tb.lines().take(10) {
                        println!("    {}", line.dimmed());
                    }
                }
            }
        }
    }
}

pub fn print_summary(results: &TestResults, verbose: bool) {
    // Group results by file
    let mut by_file: BTreeMap<PathBuf, Vec<&TestResult>> = BTreeMap::new();
    for result in &results.results {
        by_file
            .entry(result.item.file.clone())
            .or_default()
            .push(result);
    }

    // Print header
    println!("{}", "taut v0.1.0".bold());
    println!();

    // Print results grouped by file
    for (file, file_results) in &by_file {
        println!("{}", file.display().to_string().bold());
        for result in file_results {
            print_result(result, verbose);
        }
        println!();
    }

    // Print summary line
    let passed = results.passed_count();
    let failed = results.failed_count();
    let duration = results.total_duration.as_secs_f64();

    let summary = if failed == 0 {
        format!("{} passed in {:.2}s", passed, duration).green()
    } else {
        format!("{} passed, {} failed in {:.2}s", passed, failed, duration).red()
    };

    println!("{}", summary);
}

pub fn print_no_tests_found() {
    println!("{}", "taut v0.1.0".bold());
    println!();
    println!("{}", "No tests found.".yellow());
}
