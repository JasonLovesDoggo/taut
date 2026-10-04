use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use taut::discovery::TestItem;
use taut::markers::{Marker, MarkerArgs, MarkerValue};
use taut::runner::{IsolationMode, RunOptions, run_tests_with_options};
use tempfile::TempDir;

fn item(file: &Path, function: &str) -> TestItem {
    TestItem {
        file: file.into(),
        function: function.into(),
        ..TestItem::default()
    }
}
fn options() -> RunOptions {
    RunOptions {
        jobs: Some(1),
        ..RunOptions::default()
    }
}
fn suite(source: &str) -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test_cases.py");
    fs::write(&file, source).unwrap();
    (dir, file)
}

#[test]
fn warm_module_import_happens_once() {
    let (_dir, file) = suite(
        "value = 0\ndef test_one():\n global value\n value += 1\n assert value == 1\ndef test_two():\n assert value == 1\n",
    );
    let result = run_tests_with_options(
        &[item(&file, "test_one"), item(&file, "test_two")],
        &options(),
        |_| {},
    )
    .unwrap();
    assert!(result.all_passed(), "{:?}", result.results);
}

#[test]
fn private_protocol_captures_native_subprocess_and_thread_output() {
    let (_dir, file) = suite(
        "import os, subprocess, sys, threading\ndef test_output():\n os.write(1, b'native stdout\\n')\n os.write(2, b'native stderr\\n')\n subprocess.run([sys.executable, '-c', 'print(\"child stdout\")'], check=True)\n thread = threading.Thread(target=lambda: print('thread stdout'))\n thread.start()\n thread.join()\n print('python stdout')\ndef test_next(): pass\n",
    );
    let results = run_tests_with_options(
        &[item(&file, "test_output"), item(&file, "test_next")],
        &options(),
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
    let result = &results.results[0];
    for expected in [
        "native stdout",
        "child stdout",
        "thread stdout",
        "python stdout",
    ] {
        assert!(result.stdout.as_deref().unwrap().contains(expected));
    }
    assert!(result.stderr.as_deref().unwrap().contains("native stderr"));
}

#[test]
fn teardown_failure_is_failure_and_preserves_body_failure() {
    let (_dir, file) = suite(
        "class TestCase:\n def test_body(self): raise ValueError('body exploded')\n def tearDown(self): raise RuntimeError('cleanup exploded')\n",
    );
    let mut test = item(&file, "test_body");
    test.class = Some("TestCase".into());
    let results = run_tests_with_options(&[test], &options(), |_| {}).unwrap();
    let result = &results.results[0];
    assert!(!result.passed);
    let error = &result.error.as_ref().unwrap().message;
    assert!(
        error.contains("body exploded") && error.contains("cleanup exploded"),
        "{error}"
    );
}

#[test]
fn generator_tests_cannot_false_pass() {
    let (_dir, file) = suite(
        "def test_generator():\n assert False\n yield 1\nasync def test_async_generator():\n assert False\n yield 1\n",
    );
    let results = run_tests_with_options(
        &[
            item(&file, "test_generator"),
            item(&file, "test_async_generator"),
        ],
        &options(),
        |_| {},
    )
    .unwrap();
    assert_eq!(results.failed_count(), 2);
    for result in results.results {
        assert!(
            result
                .error
                .unwrap()
                .message
                .contains("Generator tests are unsupported")
        );
    }
}

#[test]
fn async_overlap_has_per_task_output_and_cancels_descendants() {
    let (_dir, file) = suite(
        "import asyncio\nactive = set()\ncleaned = set()\nasync def background(label):\n try: await asyncio.sleep(50)\n finally: cleaned.add(label)\nasync def run(label):\n active.add(label)\n child = asyncio.create_task(background(label))\n await asyncio.sleep(0.02)\n assert active == {'alpha', 'beta'}\n print(label)\n await asyncio.sleep(0.02)\n print(label)\nasync def test_alpha(): await run('alpha')\nasync def test_beta(): await run('beta')\ndef test_cleanup(): assert cleaned == {'alpha', 'beta'}\n",
    );
    let options = RunOptions {
        async_concurrency: 2,
        timeout: Some(Duration::from_secs(2)),
        ..options()
    };
    let results = run_tests_with_options(
        &[
            item(&file, "test_alpha"),
            item(&file, "test_beta"),
            item(&file, "test_cleanup"),
        ],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
    assert_eq!(results.results[0].stdout.as_deref(), Some("alpha\nalpha\n"));
    assert_eq!(results.results[1].stdout.as_deref(), Some("beta\nbeta\n"));
}

#[test]
fn no_parallel_disables_async_overlap() {
    let (_dir, file) = suite(
        "import asyncio\nactive = 0\nasync def run():\n global active\n active += 1\n await asyncio.sleep(0.01)\n assert active == 1\n active -= 1\nasync def test_a(): await run()\nasync def test_b(): await run()\n",
    );
    let options = RunOptions {
        parallel: false,
        async_concurrency: 8,
        ..options()
    };
    let results = run_tests_with_options(
        &[item(&file, "test_a"), item(&file, "test_b")],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
}

#[test]
fn sync_tests_can_run_their_own_event_loop() {
    let (_dir, file) = suite(
        "import asyncio\ndef test_loop():\n async def inner(): return 42\n assert asyncio.run(inner()) == 42\n",
    );
    assert!(
        run_tests_with_options(&[item(&file, "test_loop")], &options(), |_| {})
            .unwrap()
            .all_passed()
    );
}

#[test]
fn package_relative_imports_work() {
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("sample_package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("__init__.py"), "").unwrap();
    fs::write(package.join("helper.py"), "VALUE = 7\n").unwrap();
    let file = package.join("test_package.py");
    fs::write(
        &file,
        "from .helper import VALUE\ndef test_value(): assert VALUE == 7\n",
    )
    .unwrap();
    assert!(
        run_tests_with_options(&[item(&file, "test_value")], &options(), |_| {})
            .unwrap()
            .all_passed()
    );
}

#[test]
fn crash_is_not_retried_and_later_tests_continue() {
    let (dir, file) = suite(
        "import os\ndef test_crash():\n with open(__file__ + '.witness', 'a') as f: f.write('once')\n os._exit(9)\ndef test_next(): pass\n",
    );
    let options = RunOptions {
        timeout: Some(Duration::from_secs(2)),
        ..options()
    };
    let results = run_tests_with_options(
        &[item(&file, "test_crash"), item(&file, "test_next")],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(!results.results[0].passed);
    assert!(results.results[1].passed);
    assert_eq!(
        fs::read_to_string(dir.path().join("test_cases.py.witness")).unwrap(),
        "once"
    );
}

#[test]
fn timeout_kills_blocked_worker_and_continues() {
    let (_dir, file) =
        suite("import time\ndef test_hang(): time.sleep(30)\ndef test_next(): pass\n");
    let options = RunOptions {
        timeout: Some(Duration::from_millis(100)),
        ..options()
    };
    let start = Instant::now();
    let results = run_tests_with_options(
        &[item(&file, "test_hang"), item(&file, "test_next")],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(!results.results[0].passed);
    assert!(results.results[1].passed);
    assert!(
        results.results[0]
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("timeout")
    );
}

#[test]
fn fail_fast_omits_unstarted_tests() {
    let (_dir, file) = suite("def test_bad(): assert False\ndef test_next(): pass\n");
    let options = RunOptions {
        fail_fast: true,
        ..options()
    };
    let results = run_tests_with_options(
        &[item(&file, "test_bad"), item(&file, "test_next")],
        &options,
        |_| {},
    )
    .unwrap();
    assert_eq!(results.results.len(), 1);
    assert_eq!(results.failed_count(), 1);
}

#[test]
fn unittest_class_skip_and_runtime_skip_are_honored() {
    let (_dir, file) = suite(
        "import unittest\n@unittest.skip('class skipped')\nclass TestSkipped:\n def test_bad(self): assert False\ndef test_runtime_skip(): raise unittest.SkipTest('runtime skipped')\n",
    );
    let mut skipped = item(&file, "test_bad");
    skipped.class = Some("TestSkipped".into());
    let results = run_tests_with_options(
        &[skipped, item(&file, "test_runtime_skip")],
        &options(),
        |_| {},
    )
    .unwrap();
    assert_eq!(results.skipped_count(), 2);
    assert!(results.all_passed());
}

#[test]
fn serial_test_is_barrier_and_ordinary_tests_use_multiple_workers() {
    let (dir, file) = suite(
        "import pathlib, os, time\nroot=pathlib.Path(__file__).parent\ndef work():\n p=root / ('active-' + str(os.getpid()))\n p.touch()\n time.sleep(0.05)\n p.unlink()\n print(os.getpid())\ndef test_a(): work()\ndef test_b(): work()\ndef test_serial():\n assert not list(root.glob('active-*'))\n time.sleep(0.03)\n assert not list(root.glob('active-*'))\ndef test_c(): work()\ndef test_d(): work()\n",
    );
    let mut serial = item(&file, "test_serial");
    serial.markers.push(Marker {
        name: "mark".into(),
        args: MarkerArgs {
            kwargs: [("serial".into(), MarkerValue::Bool(true))].into(),
            ..MarkerArgs::default()
        },
    });
    let options = RunOptions {
        jobs: Some(2),
        ..options()
    };
    let results = run_tests_with_options(
        &[
            item(&file, "test_a"),
            item(&file, "test_b"),
            serial,
            item(&file, "test_c"),
            item(&file, "test_d"),
        ],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
    assert_ne!(results.results[0].stdout, results.results[1].stdout);
    drop(dir);
}

#[test]
fn healthy_shutdown_runs_atexit_but_hanging_threads_cannot_block_runner() {
    let (dir, file) = suite(
        "import atexit, pathlib, threading, time\n@atexit.register\ndef witness(): pathlib.Path(__file__+'.exit').touch()\ndef test_ok(): pass\n",
    );
    let result = run_tests_with_options(&[item(&file, "test_ok")], &options(), |_| {}).unwrap();
    assert!(result.all_passed());
    assert!(dir.path().join("test_cases.py.exit").exists());
    fs::write(&file,"import threading, time\ndef test_thread():\n threading.Thread(target=lambda: time.sleep(30)).start()\n").unwrap();
    let start = Instant::now();
    assert!(
        run_tests_with_options(&[item(&file, "test_thread")], &options(), |_| {})
            .unwrap()
            .all_passed()
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn explicit_fresh_isolation_still_resets_globals() {
    let (_dir, file) = suite(
        "state=0\ndef test_one():\n global state\n state += 1\n assert state == 1\ndef test_two():\n assert state == 0\n",
    );
    let options = RunOptions {
        isolation: IsolationMode::ProcessPerTest,
        ..options()
    };
    assert!(
        run_tests_with_options(
            &[item(&file, "test_one"), item(&file, "test_two")],
            &options,
            |_| {}
        )
        .unwrap()
        .all_passed()
    );
}

#[test]
fn missing_interpreter_is_infrastructure_error() {
    let (_dir, file) = suite("def test_ok(): pass\n");
    let options = RunOptions {
        python: Some("/nonexistent/taut-python".into()),
        ..options()
    };
    let error = run_tests_with_options(&[item(&file, "test_ok")], &options, |_| {})
        .err()
        .expect("missing interpreter must return Err");
    assert!(error.to_string().contains("Could not launch Python"));
}

#[test]
fn handled_task_exceptions_are_not_reported_as_failures() {
    let (_dir, file) = suite(
        "import asyncio\nasync def failure(): raise ValueError('expected')\nasync def test_caught():\n try: await asyncio.create_task(failure())\n except ValueError: pass\nasync def test_gather():\n values=await asyncio.gather(failure(), return_exceptions=True)\n assert isinstance(values[0], ValueError)\n",
    );
    let options = RunOptions {
        async_concurrency: 2,
        ..options()
    };
    let results = run_tests_with_options(
        &[item(&file, "test_caught"), item(&file, "test_gather")],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
}

#[test]
fn callback_and_explicit_context_task_errors_cannot_false_pass() {
    let (_dir, file) = suite(
        "import asyncio, contextvars\ndef callback(): raise AssertionError('callback failure')\nasync def failure(): raise ValueError('background failure')\nasync def test_callback(): asyncio.get_running_loop().call_soon(callback)\nasync def test_context():\n asyncio.create_task(failure(), context=contextvars.Context())\n await asyncio.sleep(0)\n",
    );
    let results = run_tests_with_options(
        &[item(&file, "test_callback"), item(&file, "test_context")],
        &options(),
        |_| {},
    )
    .unwrap();
    assert_eq!(results.failed_count(), 2, "{:?}", results.results);
}

#[test]
fn test_stdin_cannot_consume_protocol_requests() {
    let (_dir, file) = suite(
        "import os, sys\ndef test_stdin():\n assert sys.stdin.read() == ''\n assert os.read(0, 4) == b''\ndef test_next(): pass\n",
    );
    let results = run_tests_with_options(
        &[item(&file, "test_stdin"), item(&file, "test_next")],
        &options(),
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
}

#[test]
fn timers_are_cancelled_after_their_test_finishes() {
    let (_dir, file) = suite(
        "import asyncio\nchanged=False\ndef callback():\n global changed\n changed=True\nasync def test_timer(): asyncio.get_running_loop().call_later(0.03,callback)\nasync def test_next():\n await asyncio.sleep(0.05)\n assert not changed\n",
    );
    let results = run_tests_with_options(
        &[item(&file, "test_timer"), item(&file, "test_next")],
        &options(),
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
}

#[test]
fn cancellation_resistant_task_fails_and_recycles_without_rerunning_started_test() {
    let (dir, file) = suite(
        "import asyncio, pathlib\nasync def stubborn():\n while True:\n  try: await asyncio.sleep(50)\n  except asyncio.CancelledError: pass\nasync def test_leak():\n with open(__file__+'.witness', 'a') as f: f.write('once')\n asyncio.create_task(stubborn())\n await asyncio.sleep(0)\ndef test_next(): pass\n",
    );
    let mut items = vec![item(&file, "test_leak")];
    items.extend((0..8).map(|_| item(&file, "test_next")));
    let start = Instant::now();
    let results = run_tests_with_options(&items, &options(), |_| {}).unwrap();
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(results.failed_count(), 1, "{:?}", results.results);
    assert_eq!(results.passed_count(), 8);
    assert!(
        results.results[0]
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("did not stop")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("test_cases.py.witness")).unwrap(),
        "once"
    );
}

#[cfg(unix)]
#[test]
fn native_capture_fallback_uses_independent_reader_offsets() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, file) = suite(
        "import os\ndef test_one(): os.write(1, b'first')\ndef test_two(): os.write(1, b'second')\n",
    );
    let python = dir.path().join("python-no-pread");
    fs::write(
        &python,
        "#!/usr/bin/env python3\nimport os, sys\ndel os.pread\nexec(sys.argv[-1])\n",
    )
    .unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
    let options = RunOptions {
        python: Some(python),
        ..options()
    };
    let results = run_tests_with_options(
        &[item(&file, "test_one"), item(&file, "test_two")],
        &options,
        |_| {},
    )
    .unwrap();
    assert!(results.all_passed(), "{:?}", results.results);
    assert_eq!(results.results[0].stdout.as_deref(), Some("first"));
    assert_eq!(results.results[1].stdout.as_deref(), Some("second"));
}
