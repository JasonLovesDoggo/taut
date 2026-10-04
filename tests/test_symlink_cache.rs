//! Import aliases must survive persistence so retargeting a tracked link reruns tests.
#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct Project {
    root: TempDir,
    external: TempDir,
    python: PathBuf,
    import_path: PathBuf,
}

impl Project {
    fn new(test: &str) -> Self {
        let root = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        fs::write(
            root.path().join("pyproject.toml"),
            "[project]\nname='alias-tests'\n",
        )
        .unwrap();
        fs::write(root.path().join("test_alias.py"), test).unwrap();
        let python = taut::worker_pool::resolve_python(None);
        let import_path = external.path().to_path_buf();
        Self {
            root,
            external,
            python,
            import_path,
        }
    }

    fn file(&self, name: &str, source: &str) -> PathBuf {
        let path = self.external.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        path
    }

    fn run(&self, executed: usize, unchanged: usize, failed: usize) {
        let output = Command::new(env!("CARGO_BIN_EXE_taut"))
            .args(["--changed", "--json", "--python"])
            .arg(&self.python)
            .current_dir(self.root.path())
            .env("PYTHONPATH", &self.import_path)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .unwrap();
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        assert_eq!(report["summary"]["executed"], executed, "{report}");
        assert_eq!(report["summary"]["unchanged"], unchanged, "{report}");
        assert_eq!(report["summary"]["failed"], failed, "{report}");
        assert_eq!(output.status.code(), Some(if failed == 0 { 0 } else { 1 }));
    }

    fn establish_cache(&self) {
        self.run(1, 0, 0); // Discover external paths.
        self.run(1, 0, 0); // Execute against their pre-run snapshot.
        self.run(0, 1, 0); // Verify reuse is actually happening.
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_taut"))
            .args(["cache", "clear"])
            .current_dir(self.root.path())
            .output();
    }
}

fn retarget(alias: &Path, target: &Path) {
    fs::remove_file(alias).unwrap();
    symlink(target, alias).unwrap();
}

#[test]
fn retargeted_external_import_cannot_reuse_the_old_targets_pass() {
    let project =
        Project::new("def test_alias():\n    import helper\n    assert helper.VALUE == 1\n");
    let old = project.file("old.py", "VALUE = 1\n");
    let new = project.file("new.py", "VALUE = 200\n");
    let alias = project.external.path().join("helper.py");
    symlink(&old, &alias).unwrap();
    project.establish_cache();
    retarget(&alias, &new);
    project.run(1, 0, 1);
}

#[test]
fn same_byte_retarget_is_tracked_and_later_target_edits_invalidate() {
    let project =
        Project::new("def test_alias():\n    import helper\n    assert helper.VALUE == 1\n");
    let old = project.file("old.py", "VALUE = 1\n");
    let new = project.file("new.py", "VALUE = 1\n");
    let alias = project.external.path().join("helper.py");
    symlink(&old, &alias).unwrap();
    project.establish_cache();
    retarget(&alias, &new);
    project.run(1, 0, 0);
    project.run(0, 1, 0);
    fs::write(new, "VALUE = 200\n").unwrap();
    project.run(1, 0, 1);
}

#[test]
fn removed_import_alias_does_not_leave_the_old_target_reusable() {
    let project =
        Project::new("def test_alias():\n    import helper\n    assert helper.VALUE == 1\n");
    let old = project.file("old.py", "VALUE = 1\n");
    let alias = project.external.path().join("helper.py");
    symlink(&old, &alias).unwrap();
    project.establish_cache();
    fs::remove_file(alias).unwrap();
    project.run(1, 0, 1);
}

#[test]
fn directory_alias_retarget_changes_resolved_paths_even_with_identical_source() {
    let mut project = Project::new(
        "def test_alias():\n    import helper\n    assert helper.DIRECTORY == 'old'\n",
    );
    let source = "from pathlib import Path\nDIRECTORY = Path(__file__).resolve().parent.name\n";
    project.file("old/helper.py", source);
    project.file("new/helper.py", source);
    let alias = project.external.path().join("current");
    symlink(project.external.path().join("old"), &alias).unwrap();
    project.import_path = alias.clone();
    project.establish_cache();
    retarget(&alias, &project.external.path().join("new"));
    project.run(1, 0, 1);
}
