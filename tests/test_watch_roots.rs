//! Watching a test subset must observe changes to application code at its project root.
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;
use tempfile::TempDir;

struct WatchRun {
    child: Child,
    output: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl WatchRun {
    fn start(project: &TempDir, selection: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_taut"))
            .args(["watch", "--changed", "--json", selection])
            .current_dir(project.path())
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            output,
            reader: Some(reader),
        }
    }

    fn report(&self) -> serde_json::Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(10))
            .expect("watch did not produce a run after the source change");
        serde_json::from_str(&line).unwrap()
    }
}

impl Drop for WatchRun {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn project(marker: &str) -> TempDir {
    let project = TempDir::new().unwrap();
    if marker == ".git" {
        fs::create_dir(project.path().join(marker)).unwrap();
    } else {
        fs::write(project.path().join(marker), "").unwrap();
    }
    fs::create_dir(project.path().join("tests")).unwrap();
    fs::write(project.path().join("rootapp.py"), "VALUE = 1\n").unwrap();
    fs::write(
        project.path().join("tests/test_app.py"),
        "import rootapp\ndef test_value():\n    assert rootapp.VALUE == 1\n",
    )
    .unwrap();
    project
}

fn assert_application_change_is_observed(marker: &str, selection: &str) {
    let project = project(marker);
    let watch = WatchRun::start(&project, selection);
    assert_eq!(watch.report()["summary"]["passed"], 1);
    fs::write(project.path().join("rootapp.py"), "VALUE = 22\n").unwrap();
    let changed = watch.report();
    assert_eq!(changed["summary"]["executed"], 1, "{changed}");
    assert_eq!(changed["summary"]["failed"], 1, "{changed}");
    assert_eq!(changed["summary"]["unchanged"], 0, "{changed}");
}

#[test]
fn setup_project_watch_observes_application_changes_outside_selected_tests() {
    assert_application_change_is_observed("setup.cfg", "tests");
}

#[test]
fn git_project_watch_observes_application_changes_for_a_single_test_selector() {
    assert_application_change_is_observed(".git", "tests/test_app.py::test_value");
}

#[test]
fn pytest_configuration_changes_trigger_watch_runs() {
    let project = project("pytest.ini");
    fs::write(project.path().join("tests/test_app.py"), "from pathlib import Path\ndef test_value():\n    assert Path('pytest.ini').read_text() == ''\n").unwrap();
    let watch = WatchRun::start(&project, "tests");
    assert_eq!(watch.report()["summary"]["passed"], 1);
    fs::write(project.path().join("pytest.ini"), "[pytest]\n").unwrap();
    let changed = watch.report();
    assert_eq!(changed["summary"]["executed"], 1, "{changed}");
    assert_eq!(changed["summary"]["failed"], 1, "{changed}");
}
