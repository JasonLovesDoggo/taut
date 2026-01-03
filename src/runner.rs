use crate::discovery::TestItem;
use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestError {
    pub message: String,
    pub traceback: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub item: TestItem,
    pub passed: bool,
    pub duration: Duration,
    pub error: Option<TestError>,
}

pub struct TestResults {
    pub results: Vec<TestResult>,
    pub total_duration: Duration,
}

impl TestResults {
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    pub fn passed_count(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }

    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| !r.passed).count()
    }
}

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

fn run_single_test(item: &TestItem) -> TestResult {
    let start = Instant::now();

    let test_info = serde_json::json!({
        "file": item.file.to_string_lossy(),
        "function": &item.function,
        "class": &item.class,
    });

    let output = Command::new("python3")
        .args(["-c", RUNNER_SCRIPT, &test_info.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let duration = start.elapsed();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);

            if let Ok(result) = serde_json::from_str::<serde_json::Value>(&stdout) {
                TestResult {
                    item: item.clone(),
                    passed: result["passed"].as_bool().unwrap_or(false),
                    duration,
                    error: result.get("error").and_then(|e| {
                        if e.is_null() {
                            None
                        } else {
                            Some(TestError {
                                message: e["message"].as_str().unwrap_or("Unknown error").to_string(),
                                traceback: e["traceback"].as_str().map(String::from),
                            })
                        }
                    }),
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
        },
    }
}

pub fn run_tests(items: &[TestItem], parallel: bool, jobs: Option<usize>) -> Result<TestResults> {
    let start = Instant::now();

    if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok();
    }

    let results: Vec<TestResult> = if parallel && items.len() > 1 {
        items.par_iter().map(run_single_test).collect()
    } else {
        items.iter().map(run_single_test).collect()
    };

    Ok(TestResults {
        results,
        total_duration: start.elapsed(),
    })
}
