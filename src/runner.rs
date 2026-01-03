use crate::discovery::TestItem;
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestError {
    pub message: String,
    pub traceback: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TestCoverage {
    pub files: HashMap<PathBuf, Vec<usize>>,
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub item: TestItem,
    pub passed: bool,
    pub duration: Duration,
    pub error: Option<TestError>,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub coverage: Option<TestCoverage>,
}

pub struct TestResults {
    pub results: Vec<TestResult>,
    pub total_duration: Duration,
}

impl TestResults {
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed || r.skipped)
    }

    pub fn passed_count(&self) -> usize {
        self.results.iter().filter(|r| r.passed && !r.skipped).count()
    }

    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| !r.passed && !r.skipped).count()
    }

    pub fn skipped_count(&self) -> usize {
        self.results.iter().filter(|r| r.skipped).count()
    }
}

/// Basic runner script without coverage
const RUNNER_SCRIPT: &str = r#"
import sys
import json
import traceback
import importlib.util

def run_test(test_file, test_name, class_name=None):
    result = {"passed": False, "error": None}

    try:
        spec = importlib.util.spec_from_file_location("test_module", test_file)
        module = importlib.util.module_from_spec(spec)
        sys.modules["test_module"] = module
        spec.loader.exec_module(module)

        if class_name:
            cls = getattr(module, class_name)
            instance = cls()
            if hasattr(instance, "setUp"):
                instance.setUp()
            test_func = getattr(instance, test_name)
            test_func()
            if hasattr(instance, "tearDown"):
                instance.tearDown()
        else:
            test_func = getattr(module, test_name)
            test_func()

        result["passed"] = True
    except AssertionError as e:
        result["error"] = {
            "message": str(e) or "Assertion failed",
            "traceback": traceback.format_exc(),
        }
    except Exception as e:
        result["error"] = {
            "message": f"{type(e).__name__}: {e}",
            "traceback": traceback.format_exc(),
        }

    print(json.dumps(result))

if __name__ == "__main__":
    info = json.loads(sys.argv[1])
    run_test(info["file"], info["function"], info.get("class"))
"#;

/// Runner script with sys.settrace coverage collection
const RUNNER_SCRIPT_WITH_COVERAGE: &str = r#"
import sys
import json
import traceback
import importlib.util
import os

def run_test(test_file, test_name, class_name=None):
    result = {"passed": False, "error": None, "coverage": {}}
    executed_lines = {}

    def trace_function(frame, event, arg):
        if event == 'line':
            filename = frame.f_code.co_filename
            # Only track project files (skip stdlib, site-packages)
            if not any(x in filename for x in ['site-packages', 'lib/python', '/usr/lib']):
                # Normalize to absolute path
                abs_path = os.path.abspath(filename)
                if abs_path not in executed_lines:
                    executed_lines[abs_path] = set()
                executed_lines[abs_path].add(frame.f_lineno)
        return trace_function

    try:
        sys.settrace(trace_function)

        spec = importlib.util.spec_from_file_location("test_module", test_file)
        module = importlib.util.module_from_spec(spec)
        sys.modules["test_module"] = module
        spec.loader.exec_module(module)

        if class_name:
            cls = getattr(module, class_name)
            instance = cls()
            if hasattr(instance, "setUp"):
                instance.setUp()
            test_func = getattr(instance, test_name)
            test_func()
            if hasattr(instance, "tearDown"):
                instance.tearDown()
        else:
            test_func = getattr(module, test_name)
            test_func()

        result["passed"] = True
    except AssertionError as e:
        result["error"] = {
            "message": str(e) or "Assertion failed",
            "traceback": traceback.format_exc(),
        }
    except Exception as e:
        result["error"] = {
            "message": f"{type(e).__name__}: {e}",
            "traceback": traceback.format_exc(),
        }
    finally:
        sys.settrace(None)
        # Convert sets to sorted lists for JSON
        result["coverage"] = {k: sorted(v) for k, v in executed_lines.items()}

    print(json.dumps(result))

if __name__ == "__main__":
    info = json.loads(sys.argv[1])
    run_test(info["file"], info["function"], info.get("class"))
"#;

fn run_single_test(item: &TestItem, collect_coverage: bool) -> TestResult {
    let start = Instant::now();

    let test_info = serde_json::json!({
        "file": item.file.canonicalize().unwrap_or(item.file.clone()).to_string_lossy(),
        "function": &item.function,
        "class": &item.class,
    });

    let script = if collect_coverage {
        RUNNER_SCRIPT_WITH_COVERAGE
    } else {
        RUNNER_SCRIPT
    };

    let output = Command::new("python3")
        .args(["-c", script, &test_info.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let duration = start.elapsed();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);

            if let Ok(result) = serde_json::from_str::<serde_json::Value>(&stdout) {
                let coverage = if collect_coverage {
                    result.get("coverage").and_then(|c| {
                        let files: HashMap<PathBuf, Vec<usize>> = c
                            .as_object()?
                            .iter()
                            .map(|(k, v)| {
                                let path = PathBuf::from(k);
                                let lines: Vec<usize> = v
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|n| n.as_u64().map(|n| n as usize))
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                (path, lines)
                            })
                            .collect();
                        Some(TestCoverage { files })
                    })
                } else {
                    None
                };

                TestResult {
                    item: item.clone(),
                    passed: result["passed"].as_bool().unwrap_or(false),
                    duration,
                    error: result.get("error").and_then(|e| {
                        if e.is_null() {
                            None
                        } else {
                            Some(TestError {
                                message: e["message"]
                                    .as_str()
                                    .unwrap_or("Unknown error")
                                    .to_string(),
                                traceback: e["traceback"].as_str().map(String::from),
                            })
                        }
                    }),
                    skipped: false,
                    skip_reason: None,
                    coverage,
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                TestResult {
                    item: item.clone(),
                    passed: false,
                    duration,
                    error: Some(TestError {
                        message: "Failed to parse test output".to_string(),
                        traceback: Some(format!("stdout: {}\nstderr: {}", stdout, stderr)),
                    }),
                    skipped: false,
                    skip_reason: None,
                    coverage: None,
                }
            }
        }
        Err(e) => TestResult {
            item: item.clone(),
            passed: false,
            duration,
            error: Some(TestError {
                message: format!("Failed to spawn Python: {}", e),
                traceback: None,
            }),
            skipped: false,
            skip_reason: None,
            coverage: None,
        },
    }
}

/// Run tests with optional coverage collection
pub fn run_tests<F>(
    items: &[TestItem],
    parallel: bool,
    jobs: Option<usize>,
    collect_coverage: bool,
    on_result: F,
) -> Result<TestResults>
where
    F: Fn(&TestResult) + Send + Sync,
{
    use std::sync::Mutex;

    let start = Instant::now();

    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok();
    }

    let results: Vec<TestResult> = if parallel && items.len() > 1 {
        let callback = Mutex::new(&on_result);
        items
            .par_iter()
            .map(|item| {
                let result = run_single_test(item, collect_coverage);
                if let Ok(cb) = callback.lock() {
                    cb(&result);
                }
                result
            })
            .collect()
    } else {
        let mut results = Vec::new();
        for item in items {
            let result = run_single_test(item, collect_coverage);
            on_result(&result);
            results.push(result);
        }
        results
    };

    Ok(TestResults {
        results,
        total_duration: start.elapsed(),
    })
}

/// Create a skipped test result
pub fn skipped_result(item: &TestItem, reason: &str) -> TestResult {
    TestResult {
        item: item.clone(),
        passed: true,
        duration: Duration::ZERO,
        error: None,
        skipped: true,
        skip_reason: Some(reason.to_string()),
        coverage: None,
    }
}
