use crate::discovery::TestItem;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
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
    pub stdout: Option<String>,
    pub stderr: Option<String>,
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
        self.results
            .iter()
            .filter(|r| r.passed && !r.skipped)
            .count()
    }
    pub fn failed_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| !r.passed && !r.skipped)
            .count()
    }
    pub fn skipped_count(&self) -> usize {
        self.results.iter().filter(|r| r.skipped).count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationMode {
    ProcessPerTest,
    ProcessPerRun,
}

impl IsolationMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "process-per-run" => Self::ProcessPerRun,
            _ => Self::ProcessPerTest,
        }
    }
}

/// Execution policy. Warm workers import each test module once per process.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub parallel: bool,
    pub jobs: Option<usize>,
    pub collect_coverage: bool,
    pub isolation: IsolationMode,
    pub python: Option<PathBuf>,
    pub timeout: Option<Duration>,
    pub fail_fast: bool,
    /// Maximum overlapping coroutine tests in each worker (opt-in).
    pub async_concurrency: usize,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            parallel: true,
            jobs: None,
            collect_coverage: false,
            isolation: IsolationMode::ProcessPerRun,
            python: None,
            timeout: None,
            fail_fast: false,
            async_concurrency: 1,
        }
    }
}

impl RunOptions {
    pub fn python_path(&self) -> PathBuf {
        self.python
            .clone()
            .unwrap_or_else(|| crate::worker_pool::resolve_python(None))
    }

    pub(crate) fn worker_count(&self) -> usize {
        if self.parallel {
            self.jobs
                .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
        } else {
            1
        }
    }
}

/// Compatibility wrapper for callers using the original runner API.
pub fn run_tests<F>(
    items: &[TestItem],
    parallel: bool,
    jobs: Option<usize>,
    collect_coverage: bool,
    isolation: IsolationMode,
    on_result: F,
) -> Result<TestResults>
where
    F: Fn(&TestResult) + Send + Sync,
{
    run_tests_with_options(
        items,
        &RunOptions {
            parallel,
            jobs,
            collect_coverage,
            isolation,
            ..RunOptions::default()
        },
        on_result,
    )
}

pub fn run_tests_with_options<F>(
    items: &[TestItem],
    options: &RunOptions,
    on_result: F,
) -> Result<TestResults>
where
    F: Fn(&TestResult) + Send + Sync,
{
    anyhow::ensure!(
        options.jobs != Some(0),
        "worker count must be greater than zero"
    );
    anyhow::ensure!(
        options.async_concurrency > 0,
        "async concurrency must be greater than zero"
    );
    anyhow::ensure!(
        options.timeout != Some(Duration::ZERO),
        "timeout must be greater than zero"
    );
    let start = Instant::now();
    let workers = options.worker_count();
    let results = crate::worker_pool::WorkerPool::new(workers)
        .run_tests_with_options(items, options, on_result)?;
    Ok(TestResults {
        results,
        total_duration: start.elapsed(),
    })
}

pub fn skipped_result(item: &TestItem, reason: &str) -> TestResult {
    TestResult {
        item: item.clone(),
        passed: true,
        duration: Duration::ZERO,
        error: None,
        skipped: true,
        skip_reason: Some(reason.to_owned()),
        coverage: None,
        stdout: None,
        stderr: None,
    }
}

pub(crate) fn failed_result(item: &TestItem, message: String, duration: Duration) -> TestResult {
    TestResult {
        item: item.clone(),
        passed: false,
        duration,
        error: Some(TestError {
            message,
            traceback: None,
        }),
        skipped: false,
        skip_reason: None,
        coverage: None,
        stdout: None,
        stderr: None,
    }
}
