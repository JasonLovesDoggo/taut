//! Worker pool for running tests in warm Python processes.
//!
//! This module implements N long-lived Python workers that communicate via JSON-over-stdio.
//! Workers stay alive across multiple test runs, eliminating interpreter startup overhead.

use crate::discovery::TestItem;
use crate::runner::{TestCoverage, TestError, TestResult};
use anyhow::Result;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Worker script that runs as a long-lived process, reading JSON requests from stdin
/// and writing JSON responses to stdout (newline-delimited).
const WORKER_SCRIPT: &str = r#"
import sys
import json
import traceback
import importlib.util
import inspect
import asyncio
import io
import contextlib
import os
import time


def _run_maybe_async(callable_obj):
    result = callable_obj()
    if inspect.isawaitable(result):
        asyncio.run(result)


def _should_track(filename):
    if not filename or filename.startswith("<"):
        return False
    return not any(x in filename for x in ["site-packages", "lib/python", "/usr/lib"])


def _collect_coverage_with_settrace():
    executed_lines = {}

    def trace_function(frame, event, arg):
        if event == "line":
            filename = frame.f_code.co_filename
            if _should_track(filename):
                abs_path = os.path.abspath(filename)
                executed_lines.setdefault(abs_path, set()).add(frame.f_lineno)
        return trace_function

    return executed_lines, trace_function


def _collect_coverage_with_monitoring():
    mon = sys.monitoring
    executed_lines = {}
    seen_code = set()

    def on_call(code, instruction_offset):
        filename = getattr(code, "co_filename", "")
        if not _should_track(filename):
            return
        if code in seen_code:
            return
        seen_code.add(code)
        mon.set_local_events(tool_id, code, mon.events.LINE)

    def on_line(code, line_number):
        filename = getattr(code, "co_filename", "")
        if not _should_track(filename):
            return
        abs_path = os.path.abspath(filename)
        executed_lines.setdefault(abs_path, set()).add(line_number)

    tool_id = None
    for tid in range(1, mon.MAX_TOOL_ID + 1):
        try:
            mon.use_tool_id(tid, "taut_worker")
        except ValueError:
            continue
        tool_id = tid
        break

    if tool_id is None:
        raise RuntimeError("No free sys.monitoring tool id")

    mon.register_callback(tool_id, mon.events.CALL, on_call)
    mon.register_callback(tool_id, mon.events.LINE, on_line)
    mon.set_events(tool_id, mon.events.CALL)

    def uninstall():
        mon.set_events(tool_id, 0)
        mon.register_callback(tool_id, mon.events.CALL, None)
        mon.register_callback(tool_id, mon.events.LINE, None)
        mon.free_tool_id(tool_id)

    return executed_lines, uninstall


def run_test(req):
    test_file = req["file"]
    test_name = req["function"]
    class_name = req.get("class")
    collect_coverage = req.get("collect_coverage", False)
    request_id = req.get("id", 0)

    result = {
        "id": request_id,
        "passed": False,
        "error": None,
        "stdout": "",
        "stderr": "",
        "duration_sec": 0.0,
    }

    executed_lines = None
    uninstall = None
    trace_fn = None

    start = time.perf_counter()

    try:
        test_dir = os.path.dirname(os.path.abspath(test_file))
        if test_dir not in sys.path:
            sys.path.insert(0, test_dir)

        if collect_coverage:
            try:
                executed_lines, uninstall = _collect_coverage_with_monitoring()
            except Exception:
                executed_lines, trace_fn = _collect_coverage_with_settrace()
                sys.settrace(trace_fn)

        out_buf = io.StringIO()
        err_buf = io.StringIO()

        # Use unique module name to avoid cache issues
        mod_name = f"taut_test_{request_id}"

        with contextlib.redirect_stdout(out_buf), contextlib.redirect_stderr(err_buf):
            spec = importlib.util.spec_from_file_location(mod_name, test_file)
            module = importlib.util.module_from_spec(spec)
            sys.modules[mod_name] = module
            spec.loader.exec_module(module)

            if class_name:
                cls = getattr(module, class_name)
                instance = cls()
                try:
                    if hasattr(instance, "setUp"):
                        instance.setUp()
                    test_func = getattr(instance, test_name)
                    _run_maybe_async(test_func)
                    result["passed"] = True
                finally:
                    # Always run tearDown, even if test fails
                    if hasattr(instance, "tearDown"):
                        instance.tearDown()
            else:
                test_func = getattr(module, test_name)
                _run_maybe_async(test_func)
                result["passed"] = True

        # Clean up module from sys.modules
        sys.modules.pop(mod_name, None)

        result["stdout"] = out_buf.getvalue()
        result["stderr"] = err_buf.getvalue()

    except AssertionError as e:
        result["stdout"] = out_buf.getvalue() if 'out_buf' in dir() else ""
        result["stderr"] = err_buf.getvalue() if 'err_buf' in dir() else ""
        result["error"] = {"message": str(e) or "Assertion failed", "traceback": traceback.format_exc()}
    except Exception as e:
        result["stdout"] = out_buf.getvalue() if 'out_buf' in dir() else ""
        result["stderr"] = err_buf.getvalue() if 'err_buf' in dir() else ""
        result["error"] = {"message": f"{type(e).__name__}: {e}", "traceback": traceback.format_exc()}

    finally:
        if trace_fn is not None:
            sys.settrace(None)
        if uninstall is not None:
            try:
                uninstall()
            except Exception:
                pass

        if executed_lines is not None:
            result["coverage"] = {k: sorted(v) for k, v in executed_lines.items()}

        result["duration_sec"] = time.perf_counter() - start

    return result


def main():
    # Ensure unbuffered output
    sys.stdout.reconfigure(line_buffering=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            req = json.loads(line)

            if req.get("cmd") == "shutdown":
                break

            if req.get("cmd") == "ping":
                print(json.dumps({"id": req.get("id", 0), "pong": True}), flush=True)
                continue

            resp = run_test(req)

        except Exception as e:
            resp = {
                "id": req.get("id", -1) if isinstance(req, dict) else -1,
                "passed": False,
                "error": {"message": f"Worker error: {e}", "traceback": traceback.format_exc()},
                "stdout": "",
                "stderr": "",
                "duration_sec": 0.0,
            }

        print(json.dumps(resp), flush=True)


if __name__ == "__main__":
    main()
"#;

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::SeqCst)
}

/// A single Python worker process.
struct Worker {
    child: Child,
    stdin: BufWriter<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Worker {
    fn spawn() -> Result<Self> {
        let mut child = Command::new("python3")
            .args(["-u", "-c", WORKER_SCRIPT])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) // Let Python errors go to terminal
            .spawn()?;

        let stdin = BufWriter::new(child.stdin.take().expect("stdin not captured"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout not captured"));

        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    fn send_request(&mut self, req: &serde_json::Value) -> Result<()> {
        let line = serde_json::to_string(req)?;
        writeln!(self.stdin, "{}", line)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line)?;
        if n == 0 {
            anyhow::bail!("Worker EOF (process died)");
        }
        let resp: serde_json::Value = serde_json::from_str(&line)?;
        Ok(resp)
    }

    fn run_test(&mut self, item: &TestItem, collect_coverage: bool) -> Result<TestResult> {
        let request_id = next_request_id();
        let start = Instant::now();

        let req = serde_json::json!({
            "id": request_id,
            "file": item.file.canonicalize().unwrap_or(item.file.clone()).to_string_lossy(),
            "function": &item.function,
            "class": &item.class,
            "collect_coverage": collect_coverage,
        });

        self.send_request(&req)?;
        let resp = self.read_response()?;

        let duration = Duration::from_secs_f64(
            resp.get("duration_sec")
                .and_then(|v| v.as_f64())
                .unwrap_or(start.elapsed().as_secs_f64()),
        );

        let coverage = if collect_coverage {
            resp.get("coverage").and_then(|c| {
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

        Ok(TestResult {
            item: item.clone(),
            passed: resp
                .get("passed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            duration,
            error: resp.get("error").and_then(|e| {
                if e.is_null() {
                    None
                } else {
                    Some(TestError {
                        message: e
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown error")
                            .to_string(),
                        traceback: e
                            .get("traceback")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                }
            }),
            skipped: false,
            skip_reason: None,
            coverage,
            stdout: resp
                .get("stdout")
                .and_then(|v| v.as_str().map(String::from)),
            stderr: resp
                .get("stderr")
                .and_then(|v| v.as_str().map(String::from)),
        })
    }

    fn shutdown(&mut self) {
        let _ = self.send_request(&serde_json::json!({"cmd": "shutdown"}));
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

        // Channel to collect results
        let (tx, rx): (Sender<Completed>, Receiver<Completed>) = channel();

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
