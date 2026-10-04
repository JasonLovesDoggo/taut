//! A run finishes only after healthy workers complete Python shutdown.
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn project(source: &str) -> TempDir {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("test_shutdown.py"), source).unwrap();
    project
}

fn run(project: &Path, args: &[&str], python_path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_taut"));
    command
        .args(["--json", "-j", "1"])
        .args(args)
        .current_dir(project)
        .env_remove("VIRTUAL_ENV")
        .env_remove("TAUT_PYTHON")
        .env_remove("PYTHONPATH")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = python_path {
        command.env("PYTHONPATH", path);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("runner did not bound shutdown: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn report(output: &Output, code: i32) -> Value {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}: {output:?}"))
}

fn shutdown_error(output: &Output, expected: &str) {
    let report = report(output, 2);
    assert_eq!(report["schema_version"], 1);
    let error = report["error"].as_str().unwrap();
    assert!(
        error.contains("shutdown") && error.contains(expected),
        "{report}"
    );
    assert!(report.get("tests").is_none(), "{report}");
    assert!(report.get("summary").is_none(), "{report}");
}

#[test]
fn slow_exit_handlers_finish_before_normal_and_isolated_runs_return() {
    for isolation in ["process-per-run", "process-per-test"] {
        let project = project(
            r#"import atexit, json, os, time
from pathlib import Path
completed = []
@atexit.register
def flush():
    with Path(str(os.getpid()) + '.json').open('w') as stream:
        time.sleep(0.35)
        json.dump(completed, stream)
def test_one(): completed.append('one')
def test_two(): completed.append('two')
"#,
        );
        let output = run(project.path(), &["--isolation", isolation], None);
        assert_eq!(report(&output, 0)["summary"]["passed"], 2);
        let records: Vec<Vec<String>> = fs::read_dir(project.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|path| serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
            .collect();
        assert_eq!(
            records.len(),
            if isolation == "process-per-test" {
                2
            } else {
                1
            }
        );
        let mut completed: Vec<_> = records.into_iter().flatten().collect();
        completed.sort();
        assert_eq!(completed, ["one", "two"]);
    }
}

#[test]
fn fail_fast_still_finishes_exit_handlers() {
    let project = project(
        "import atexit, time\nfrom pathlib import Path\n@atexit.register\ndef flush():\n    time.sleep(0.35)\n    Path('finished').touch()\ndef test_first(): assert False\ndef test_second(): Path('must-not-run').touch()\n",
    );
    let output = run(project.path(), &["-x"], None);
    let report = report(&output, 1);
    assert_eq!(report["summary"]["failed"], 1);
    assert_eq!(report["summary"]["not_run"], 1);
    assert!(project.path().join("finished").exists());
    assert!(!project.path().join("must-not-run").exists());
}

#[test]
fn abnormal_shutdown_is_a_runner_error_on_every_healthy_exit_path() {
    for args in [vec![], vec!["--isolation", "process-per-test"], vec!["-x"]] {
        let failure = if args.contains(&"-x") {
            "assert False"
        } else {
            "pass"
        };
        let project = project(&format!(
            "import atexit, os\natexit.register(lambda: os._exit(23))\ndef test_first(): {failure}\ndef test_second(): pass\n"
        ));
        shutdown_error(&run(project.path(), &args, None), "23");
    }
}

#[test]
fn doctor_checks_worker_shutdown_before_reporting_success() {
    let project = project("def test_unused(): pass\n");
    fs::write(
        project.path().join("sitecustomize.py"),
        "import atexit, os\natexit.register(lambda: os._exit(24))\n",
    )
    .unwrap();
    shutdown_error(
        &run(project.path(), &["doctor"], Some(project.path())),
        "24",
    );
}

#[test]
fn hung_exit_handler_is_bounded_and_owned_descendants_are_cleaned_up() {
    let project = project(
        r#"import atexit, os, subprocess, sys, time
from pathlib import Path
@atexit.register
def hang():
    Path('shutdown-started').touch()
    time.sleep(30)
    Path('must-not-finish').touch()
def test_child():
    if os.name == 'posix':
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        Path('child.pid').write_text(str(child.pid))
"#,
    );
    let started = Instant::now();
    let output = run(project.path(), &[], None);
    shutdown_error(&output, "timed out after 5 seconds");
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(project.path().join("shutdown-started").exists());
    assert!(!project.path().join("must-not-finish").exists());
    #[cfg(unix)]
    {
        let pid: libc::pid_t = fs::read_to_string(project.path().join("child.pid"))
            .unwrap()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: signal zero checks only the child recorded by this test.
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            #[cfg(target_os = "linux")]
            let zombie = fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| stat.split_whitespace().nth(2) == Some("Z"));
            #[cfg(not(target_os = "linux"))]
            let zombie = false;
            if !alive || zombie {
                break;
            }
            if Instant::now() >= deadline {
                // SAFETY: clean up the still-running child owned by this test.
                unsafe { libc::kill(pid, libc::SIGKILL) };
                panic!("worker descendant {pid} survived shutdown timeout");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
