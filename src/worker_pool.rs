//! Warm Python workers, with a dependency-free framed JSON protocol.
//! A private Python descriptor carries replies so test writes to fd 1 cannot corrupt it.

use crate::discovery::TestItem;
use crate::markers::MarkerValue;
use crate::runner::{
    IsolationMode, RunOptions, TestCoverage, TestError, TestResult, failed_result, skipped_result,
};
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, unbounded};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/worker_script.rs"));
const MAX_FRAME: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct WorkerResponse {
    id: usize,
    passed: bool,
    error: Option<TestError>,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    duration_sec: f64,
    #[serde(default)]
    skipped: bool,
    skip_reason: Option<String>,
    coverage: Option<HashMap<PathBuf, Vec<usize>>>,
}

/// Locate the interpreter belonging to the invoked environment without starting Python.
pub fn resolve_python(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Some(path) = std::env::var_os("TAUT_PYTHON") {
        return path.into();
    }
    let relative = if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    };
    if let Some(venv) = std::env::var_os("VIRTUAL_ENV") {
        let path = PathBuf::from(venv).join(relative);
        if path.is_file() {
            return path;
        }
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            let path = directory.join(if cfg!(windows) {
                "python.exe"
            } else {
                "python"
            });
            if path.is_file() {
                return path;
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for directory in cwd.ancestors() {
            let path = directory.join(".venv").join(relative);
            if path.is_file() {
                return path;
            }
        }
    }
    PathBuf::from(if cfg!(windows) { "python" } else { "python3" })
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<Result<serde_json::Value>>,
    reader: Option<JoinHandle<()>>,
    healthy: bool,
}

impl Worker {
    fn spawn(python: &PathBuf) -> Result<Self> {
        let mut child = Command::new(python)
            .args(["-u", "-c", WORKER_SCRIPT])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("Could not launch Python at {}", python.display()))?;
        let stdin = child.stdin.take().context("Worker stdin unavailable")?;
        let mut stdout = child.stdout.take().context("Worker stdout unavailable")?;
        let (tx, responses) = unbounded();
        let reader = thread::spawn(move || {
            loop {
                let response = (|| -> Result<serde_json::Value> {
                    let mut length = [0; 4];
                    stdout
                        .read_exact(&mut length)
                        .context("Python worker exited before returning a result")?;
                    let length = u32::from_le_bytes(length) as usize;
                    anyhow::ensure!(length <= MAX_FRAME, "Worker response exceeds 64 MiB");
                    let mut data = vec![0; length];
                    stdout
                        .read_exact(&mut data)
                        .context("Incomplete Python worker response")?;
                    Ok(serde_json::from_slice(&data)?)
                })();
                let failed = response.is_err();
                if tx.send(response).is_err() || failed {
                    break;
                }
            }
        });
        let mut worker = Self {
            child,
            stdin,
            responses,
            reader: Some(reader),
            healthy: false,
        };
        let ready = worker
            .responses
            .recv_timeout(Duration::from_secs(30))
            .context("Python worker did not start within 30 seconds")??;
        anyhow::ensure!(
            ready.get("ready").and_then(|v| v.as_bool()) == Some(true),
            "Invalid worker greeting"
        );
        worker.healthy = true;
        Ok(worker)
    }

    fn run_batch(
        &mut self,
        batch: &[(usize, &TestItem)],
        options: &RunOptions,
        mut completed: impl FnMut(usize, TestResult),
    ) -> Result<bool> {
        let tests: Vec<_> = batch.iter().map(|(idx, item)| serde_json::json!({
            "id": idx, "file": item.file.canonicalize().unwrap_or_else(|_| item.file.clone()),
            "function": item.function, "class": item.class,
        })).collect();
        let data = serde_json::to_vec(&serde_json::json!({
            "tests": tests, "collect_coverage": options.collect_coverage,
            "async_concurrency": if options.parallel { options.async_concurrency } else { 1 },
            "timeout": options.timeout.map(|timeout| timeout.as_secs_f64()),
        }))?;
        self.stdin.write_all(&(data.len() as u32).to_le_bytes())?;
        self.stdin.write_all(&data)?;
        self.stdin.flush()?;
        let mut remaining: HashMap<usize, &TestItem> = batch.iter().copied().collect();
        loop {
            let raw = if let Some(timeout) = options.timeout {
                self.responses
                    .recv_timeout(timeout)
                    .map_err(|error| match error {
                        crossbeam_channel::RecvTimeoutError::Timeout => anyhow::anyhow!(
                            "Test exceeded timeout of {:.3}s; worker terminated",
                            timeout.as_secs_f64()
                        ),
                        _ => anyhow::anyhow!("Python worker response channel closed"),
                    })??
            } else {
                self.responses
                    .recv()
                    .context("Python worker response channel closed")??
            };
            if raw.get("done").and_then(|value| value.as_bool()) == Some(true) {
                let restart = raw
                    .get("restart")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                anyhow::ensure!(
                    remaining.is_empty() || restart,
                    "Worker ended batch before all tests completed"
                );
                return Ok(restart);
            }
            let response: WorkerResponse = serde_json::from_value(raw)?;
            let item = remaining
                .remove(&response.id)
                .context("Unexpected or duplicate worker response ID")?;
            anyhow::ensure!(
                response.duration_sec.is_finite() && response.duration_sec >= 0.0,
                "Invalid worker duration"
            );
            completed(
                response.id,
                TestResult {
                    item: item.clone(),
                    passed: response.passed,
                    duration: Duration::from_secs_f64(response.duration_sec),
                    error: response.error,
                    skipped: response.skipped,
                    skip_reason: response.skip_reason,
                    coverage: response.coverage.map(|files| TestCoverage { files }),
                    stdout: (!response.stdout.is_empty()).then_some(response.stdout),
                    stderr: (!response.stderr.is_empty()).then_some(response.stderr),
                },
            );
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.healthy {
            let shutdown = b"{\"cmd\":\"shutdown\"}";
            let sent = self
                .stdin
                .write_all(&(shutdown.len() as u32).to_le_bytes())
                .and_then(|_| self.stdin.write_all(shutdown))
                .and_then(|_| self.stdin.flush());
            if sent.is_ok() {
                let deadline = Instant::now() + Duration::from_millis(250);
                while Instant::now() < deadline {
                    if matches!(self.child.try_wait(), Ok(Some(_))) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Descendant processes may have inherited the pipe. Never block shutdown on them.
        if self
            .reader
            .as_ref()
            .is_some_and(|reader| reader.is_finished())
        {
            let _ = self.reader.take().unwrap().join();
        }
    }
}

struct QueueState {
    next: usize,
    active: usize,
    serial_active: bool,
    stopped: bool,
    fatal_error: Option<String>,
}
struct Queue {
    state: Mutex<QueueState>,
    changed: Condvar,
}

fn is_serial(item: &TestItem) -> bool {
    item.markers.iter().any(|marker| {
        marker.name == "mark"
            && matches!(
                marker.args.kwargs.get("serial"),
                Some(MarkerValue::Bool(true))
            )
    })
}

/// Each worker owns a Python process for the entire run, including serial barriers.
pub struct WorkerPool {
    num_workers: usize,
}

impl WorkerPool {
    pub fn new(num_workers: usize) -> Self {
        Self {
            num_workers: num_workers.max(1),
        }
    }

    pub fn run_tests<F>(
        &self,
        items: &[TestItem],
        collect_coverage: bool,
        on_result: F,
    ) -> Result<Vec<TestResult>>
    where
        F: Fn(&TestResult) + Send + Sync,
    {
        self.run_tests_with_options(
            items,
            &RunOptions {
                jobs: Some(self.num_workers),
                collect_coverage,
                ..RunOptions::default()
            },
            on_result,
        )
    }

    pub fn run_tests_with_options<F>(
        &self,
        items: &[TestItem],
        options: &RunOptions,
        on_result: F,
    ) -> Result<Vec<TestResult>>
    where
        F: Fn(&TestResult) + Send + Sync,
    {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let workers = self.num_workers.min(items.len());
        let batch_size = if options.isolation == IsolationMode::ProcessPerTest
            || options.fail_fast
            || options.timeout.is_some()
        {
            if options.isolation == IsolationMode::ProcessPerTest || options.fail_fast {
                1
            } else {
                options.async_concurrency
            }
        } else {
            (items.len() / (workers * 4))
                .clamp(1, 32)
                .max(options.async_concurrency)
        };
        let queue = Queue {
            state: Mutex::new(QueueState {
                next: 0,
                active: 0,
                serial_active: false,
                stopped: false,
                fatal_error: None,
            }),
            changed: Condvar::new(),
        };
        let (tx, rx) = unbounded();
        let python = options.python_path();
        let mut results: Vec<Option<TestResult>> = vec![None; items.len()];
        thread::scope(|scope| {
            for _ in 0..workers {
                let tx = tx.clone();
                let queue = &queue;
                let python = &python;
                scope.spawn(move || worker_thread(items, options, python, batch_size, queue, tx));
            }
            drop(tx);
            for (idx, result) in rx {
                on_result(&result);
                results[idx] = Some(result);
            }
        });
        if let Some(error) = queue.state.lock().unwrap().fatal_error.take() {
            anyhow::bail!(error);
        }
        Ok(results.into_iter().flatten().collect())
    }
}

fn worker_thread(
    items: &[TestItem],
    options: &RunOptions,
    python: &PathBuf,
    batch_size: usize,
    queue: &Queue,
    tx: Sender<(usize, TestResult)>,
) {
    let mut worker: Option<Worker> = None;
    loop {
        let (batch, serial) = {
            let mut state = queue.state.lock().unwrap();
            loop {
                if state.stopped || state.next == items.len() {
                    return;
                }
                let serial = is_serial(&items[state.next]);
                if !state.serial_active && (!serial || state.active == 0) {
                    break;
                }
                state = queue.changed.wait(state).unwrap();
            }
            let start = state.next;
            let serial = is_serial(&items[start]);
            state.next += 1;
            if !serial {
                while state.next < items.len()
                    && state.next - start < batch_size
                    && !is_serial(&items[state.next])
                {
                    state.next += 1;
                }
            }
            state.active += 1;
            state.serial_active = serial;
            ((start..state.next).collect::<Vec<_>>(), serial)
        };
        let mut pending = Vec::new();
        for idx in batch {
            if items[idx].is_skipped() {
                let result =
                    skipped_result(&items[idx], &items[idx].skip_reason().unwrap_or_default());
                let _ = tx.send((idx, result));
            } else {
                pending.push((idx, &items[idx]));
            }
        }
        while !pending.is_empty() {
            let start = Instant::now();
            let mut done = Vec::new();
            if worker.is_none() {
                match Worker::spawn(python) {
                    Ok(started) => worker = Some(started),
                    Err(error) => {
                        let mut state = queue.state.lock().unwrap();
                        state.stopped = true;
                        state.fatal_error = Some(format!("{error:#}"));
                        queue.changed.notify_all();
                        return;
                    }
                }
            }
            let execution = (|| -> Result<bool> {
                worker
                    .as_mut()
                    .unwrap()
                    .run_batch(&pending, options, |idx, result| {
                        done.push(idx);
                        if options.fail_fast && !result.passed && !result.skipped {
                            queue.state.lock().unwrap().stopped = true;
                            queue.changed.notify_all();
                        }
                        let _ = tx.send((idx, result));
                    })
            })();
            pending.retain(|(idx, _)| !done.contains(idx));
            match execution {
                Ok(restart) => {
                    if restart {
                        worker.as_mut().unwrap().healthy = false;
                        worker.take();
                    }
                }
                Err(error) => {
                    if let Some(worker) = worker.as_mut() {
                        worker.healthy = false;
                    }
                    worker.take();
                    if pending.is_empty() {
                        let mut state = queue.state.lock().unwrap();
                        state.stopped = true;
                        state.fatal_error =
                            Some(format!("Worker failed to finish its batch: {error:#}"));
                    }
                    for (idx, item) in pending.drain(..) {
                        let _ = tx.send((
                            idx,
                            failed_result(item, format!("{error:#}"), start.elapsed()),
                        ));
                    }
                    if options.fail_fast {
                        queue.state.lock().unwrap().stopped = true;
                    }
                }
            }
            if options.isolation == IsolationMode::ProcessPerTest {
                worker.take();
            }
        }
        let mut state = queue.state.lock().unwrap();
        state.active -= 1;
        if serial {
            state.serial_active = false;
        }
        queue.changed.notify_all();
    }
}
