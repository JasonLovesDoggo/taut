//! Worker pool for running tests in warm Python processes.
//!
//! This module implements N long-lived Python workers that communicate via MessagePack-over-stdio
//! with length-prefixed binary protocol. Workers stay alive across multiple test runs, eliminating
//! interpreter startup overhead.

use crate::discovery::TestItem;
use crate::runner::{TestCoverage, TestError, TestResult};
use anyhow::Result;
use crossbeam_channel::{Sender, bounded};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

// Worker script is embedded at build time from src/worker.py
include!(concat!(env!("OUT_DIR"), "/worker_script.rs"));

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::SeqCst)
}

/// Request sent to worker (serialized as MessagePack).
#[derive(Serialize, Deserialize, Clone)]
struct WorkerRequest {
    id: u64,
    file: String,
    function: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    collect_coverage: bool,
}

/// Response from worker (serialized as MessagePack).
#[derive(Serialize, Deserialize)]
struct WorkerResponse {
    id: u64,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<WorkerError>,
    stdout: String,
    stderr: String,
    duration_sec: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<HashMap<String, Vec<usize>>>,
}

#[derive(Serialize, Deserialize)]
struct WorkerError {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    traceback: Option<String>,
}

/// A single Python worker process.
struct Worker {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: std::process::ChildStdout,
}

impl Worker {
    fn spawn() -> Result<Self> {
        let mut child = Command::new("python3")
            .args(["-u", "-c", WORKER_SCRIPT])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) // Let Python errors go to terminal
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin not captured");
        let stdout = child.stdout.take().expect("stdout not captured");

        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    fn send_request(&mut self, req: &WorkerRequest) -> Result<()> {
        let data = rmp_serde::to_vec(req)?;
        let len = (data.len() as u32).to_le_bytes();
        self.stdin.write_all(&len)?;
        self.stdin.write_all(&data)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<WorkerResponse> {
        let mut len_bytes = [0u8; 4];
        if self.stdout.read_exact(&mut len_bytes).is_err() {
            anyhow::bail!("Worker EOF (process died)");
        }
        let len = u32::from_le_bytes(len_bytes) as usize;

        let mut data = vec![0u8; len];
        self.stdout.read_exact(&mut data)?;

        let resp: WorkerResponse = rmp_serde::from_slice(&data)?;
        Ok(resp)
    }

    fn run_test(&mut self, item: &TestItem, collect_coverage: bool) -> Result<TestResult> {
        let request_id = next_request_id();

        let req = WorkerRequest {
            id: request_id,
            file: item
                .file
                .canonicalize()
                .unwrap_or(item.file.clone())
                .to_string_lossy()
                .into_owned(),
            function: item.function.clone(),
            class: item.class.clone(),
            collect_coverage,
        };

        self.send_request(&req)?;
        let resp = self.read_response()?;

        let duration = Duration::from_secs_f64(resp.duration_sec);

        let coverage = if collect_coverage {
            resp.coverage.as_ref().map(|coverage_map| {
                let files: HashMap<PathBuf, Vec<usize>> = coverage_map
                    .iter()
                    .map(|(k, v)| (PathBuf::from(k), v.clone()))
                    .collect();
                TestCoverage { files }
            })
        } else {
            None
        };

        let error = resp.error.map(|e| TestError {
            message: e.message,
            traceback: e.traceback,
        });

        Ok(TestResult {
            item: item.clone(),
            passed: resp.passed,
            duration,
            error,
            skipped: false,
            skip_reason: None,
            coverage,
            stdout: if resp.stdout.is_empty() {
                None
            } else {
                Some(resp.stdout)
            },
            stderr: if resp.stderr.is_empty() {
                None
            } else {
                Some(resp.stderr)
            },
        })
    }

    fn shutdown(&mut self) {
        // Send shutdown command as MessagePack
        let mut shutdown_msg = std::collections::HashMap::new();
        shutdown_msg.insert("cmd", "shutdown");
        if let Ok(data) = rmp_serde::to_vec(&shutdown_msg) {
            let len = (data.len() as u32).to_le_bytes();
            let _ = self.stdin.write_all(&len);
            let _ = self.stdin.write_all(&data);
            let _ = self.stdin.flush();
        }
        let _ = self.child.wait();
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Task to be executed by a worker.
struct Task {
    idx: usize,
    item: TestItem,
    collect_coverage: bool,
}

/// Completed task result.
struct Completed {
    idx: usize,
    result: TestResult,
}

/// A pool of warm Python workers.
pub struct WorkerPool {
    num_workers: usize,
}

impl WorkerPool {
    pub fn new(num_workers: usize) -> Self {
        Self { num_workers }
    }

    /// Run tests using the worker pool.
    pub fn run_tests<F>(
        &self,
        items: &[TestItem],
        collect_coverage: bool,
        on_result: F,
    ) -> Result<Vec<TestResult>>
    where
        F: Fn(&TestResult) + Send + Sync,
    {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        // For small test counts, just use a single worker
        let num_workers = self.num_workers.min(items.len());

        // Create a shared work queue
        let queue: Arc<(Mutex<std::collections::VecDeque<Task>>, Condvar)> = Arc::new((
            Mutex::new(std::collections::VecDeque::new()),
            Condvar::new(),
        ));

        // Populate the queue
        {
            let (lock, cvar) = &*queue;
            let mut q = lock.lock().unwrap();
            for (idx, item) in items.iter().enumerate() {
                q.push_back(Task {
                    idx,
                    item: item.clone(),
                    collect_coverage,
                });
            }
            cvar.notify_all();
        }

        // Channel to collect results (bounded to number of items for backpressure)
        let (tx, rx) = bounded::<Completed>(items.len().max(1));

        // Spawn worker threads
        let mut handles = Vec::with_capacity(num_workers);
        for _ in 0..num_workers {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            let total_tasks = items.len();

            handles.push(thread::spawn(move || {
                worker_thread(queue, tx, total_tasks);
            }));
        }

        // Drop our sender so rx closes when all workers finish
        drop(tx);

        // Collect results with streaming callback
        let on_result = Arc::new(on_result);
        let mut results_by_idx: Vec<Option<TestResult>> = vec![None; items.len()];
        let mut received = 0;

        for completed in rx {
            on_result(&completed.result);
            results_by_idx[completed.idx] = Some(completed.result);
            received += 1;
            if received >= items.len() {
                break;
            }
        }

        // Wait for all worker threads to finish
        for handle in handles {
            let _ = handle.join();
        }

        // Collect results in order
        let results = results_by_idx
            .into_iter()
            .enumerate()
            .map(|(idx, opt)| {
                opt.unwrap_or_else(|| TestResult {
                    item: items[idx].clone(),
                    passed: false,
                    duration: Duration::ZERO,
                    error: Some(TestError {
                        message: "Test was not executed (worker pool error)".to_string(),
                        traceback: None,
                    }),
                    skipped: false,
                    skip_reason: None,
                    coverage: None,
                    stdout: None,
                    stderr: None,
                })
            })
            .collect();

        Ok(results)
    }
}

fn worker_thread(
    queue: Arc<(Mutex<std::collections::VecDeque<Task>>, Condvar)>,
    tx: Sender<Completed>,
    total_tasks: usize,
) {
    let mut worker = match Worker::spawn() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to spawn worker: {}", e);
            return;
        }
    };

    let mut tasks_completed = 0;

    loop {
        // Try to get a task from the queue
        let task = {
            let (lock, _cvar) = &*queue;
            let mut q = lock.lock().unwrap();
            q.pop_front()
        };

        let Some(task) = task else {
            // No more tasks
            break;
        };

        // Execute the task
        let result = match worker.run_test(&task.item, task.collect_coverage) {
            Ok(r) => r,
            Err(e) => {
                // Worker might have died; try to respawn
                if !worker.is_alive() {
                    if let Ok(new_worker) = Worker::spawn() {
                        worker = new_worker;
                        // Retry the test
                        match worker.run_test(&task.item, task.collect_coverage) {
                            Ok(r) => r,
                            Err(e2) => TestResult {
                                item: task.item.clone(),
                                passed: false,
                                duration: Duration::ZERO,
                                error: Some(TestError {
                                    message: format!("Worker error after respawn: {}", e2),
                                    traceback: None,
                                }),
                                skipped: false,
                                skip_reason: None,
                                coverage: None,
                                stdout: None,
                                stderr: None,
                            },
                        }
                    } else {
                        TestResult {
                            item: task.item.clone(),
                            passed: false,
                            duration: Duration::ZERO,
                            error: Some(TestError {
                                message: format!("Worker crashed and respawn failed: {}", e),
                                traceback: None,
                            }),
                            skipped: false,
                            skip_reason: None,
                            coverage: None,
                            stdout: None,
                            stderr: None,
                        }
                    }
                } else {
                    TestResult {
                        item: task.item.clone(),
                        passed: false,
                        duration: Duration::ZERO,
                        error: Some(TestError {
                            message: format!("Worker error: {}", e),
                            traceback: None,
                        }),
                        skipped: false,
                        skip_reason: None,
                        coverage: None,
                        stdout: None,
                        stderr: None,
                    }
                }
            }
        };

        // Send result back
        if tx
            .send(Completed {
                idx: task.idx,
                result,
            })
            .is_err()
        {
            break;
        }

        tasks_completed += 1;

        // Early exit if we've done all tasks
        if tasks_completed >= total_tasks {
            break;
        }
    }

    worker.shutdown();
}
