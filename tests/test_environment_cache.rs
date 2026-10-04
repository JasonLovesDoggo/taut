//! Installed distribution metadata is tracked without traversing package code.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use taut::discovery::TestItem;
use taut::runner::{TestCoverage, TestResult};
use taut::selection::TestSelector;
use tempfile::TempDir;

struct Project {
    root: TempDir,
    test: TestItem,
    selector: TestSelector,
}

impl Project {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        fs::write(
            root.path().join("pyproject.toml"),
            "[project]\nname='example'\n",
        )
        .unwrap();
        let file = root.path().join("test_example.py");
        fs::write(&file, "def test_example(): pass\n").unwrap();
        Self {
            root,
            test: TestItem {
                file,
                function: "test_example".into(),
                class: None,
                line: 1,
                markers: vec![],
            },
            selector: TestSelector::new(),
        }
    }

    fn index(&mut self) {
        self.selector
            .index_files(std::slice::from_ref(&self.test.file));
    }

    fn pass(&mut self) {
        self.index();
        self.selector.record_result(&TestResult {
            item: self.test.clone(),
            passed: true,
            duration: Duration::ZERO,
            error: None,
            skipped: false,
            skip_reason: None,
            stdout: None,
            stderr: None,
            coverage: Some(TestCoverage {
                files: HashMap::from([(self.test.file.clone(), vec![1])]),
            }),
        });
        assert_eq!(
            self.selector
                .select_tests(std::slice::from_ref(&self.test))
                .skip_count(),
            1
        );
    }

    fn assert_changed(&mut self) {
        self.index();
        assert_eq!(
            self.selector
                .select_tests(std::slice::from_ref(&self.test))
                .run_count(),
            1
        );
    }
}

fn environment(root: &Path, layout: &str) -> PathBuf {
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("pyvenv.cfg"), "home = /python\nversion = 3.13\n").unwrap();
    let packages = root.join(layout);
    fs::create_dir_all(&packages).unwrap();
    packages
}

fn write(path: &Path, bytes: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn conventional_environments_track_metadata_additions_edits_and_deletions() {
    for (name, layout) in [
        (".venv", "lib/python3.13/site-packages"),
        ("venv", "Lib/site-packages"),
    ] {
        let mut project = Project::new();
        let packages = environment(&project.root.path().join(name), layout);
        project.pass();
        for relative in [
            "example-1.0.dist-info/METADATA",
            "example-1.0.dist-info/RECORD",
            "example-1.0.dist-info/direct_url.json",
            "example.egg-info/PKG-INFO",
            "legacy.egg-info",
            "editable.pth",
            "editable.egg-link",
        ] {
            let metadata = packages.join(relative);
            write(&metadata, "version one\n");
            project.assert_changed();
            project.pass();
            write(&metadata, "version two\n");
            project.assert_changed();
            project.pass();
            fs::remove_file(metadata).unwrap();
            project.assert_changed();
            project.pass();
        }
    }
}

#[test]
fn virtual_environment_configuration_change_invalidates_cache() {
    let mut project = Project::new();
    let root = project.root.path().join(".venv");
    environment(&root, "lib/python3.13/site-packages");
    project.pass();
    fs::write(
        root.join("pyvenv.cfg"),
        "include-system-site-packages = true\n",
    )
    .unwrap();
    project.assert_changed();
    project.pass();
    fs::remove_file(root.join("pyvenv.cfg")).unwrap();
    project.assert_changed();
}

#[test]
fn explicit_external_interpreter_tracks_its_environment_without_resolving_symlink() {
    let mut project = Project::new();
    let external = TempDir::new().unwrap();
    let packages = environment(external.path(), "lib/python3.13/site-packages");
    let interpreter = external.path().join("bin/python");
    #[cfg(unix)]
    std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &interpreter).unwrap();
    #[cfg(not(unix))]
    fs::write(&interpreter, "fake interpreter").unwrap();
    project.selector.set_python_environment(&interpreter);
    project.pass();
    write(&packages.join("example.dist-info/METADATA"), "Version: 2\n");
    project.assert_changed();
}

#[test]
fn package_source_files_are_not_walked_or_claimed_as_metadata_inputs() {
    let mut project = Project::new();
    let packages = environment(
        &project.root.path().join(".venv"),
        "lib/python3.13/site-packages",
    );
    let code = packages.join("example/__init__.py");
    write(&code, "VALUE = 1\n");
    write(&packages.join("example.dist-info/METADATA"), "Version: 1\n");
    project.pass();
    write(&code, "VALUE = 2\n");
    project.index();
    // Manual edits without distribution metadata changes require a full run.
    assert_eq!(
        project.selector.select_tests(&[project.test]).skip_count(),
        1
    );
}

#[test]
fn activated_environment_metadata_is_tracked() {
    const CHILD_MARKER: &str = "TAUT_ACTIVATED_ENV_CACHE_TEST";
    if std::env::var_os(CHILD_MARKER).is_some() {
        let mut project = Project::new();
        project.pass();
        let environment = PathBuf::from(std::env::var_os("VIRTUAL_ENV").unwrap());
        write(
            &environment.join("lib/python3.13/site-packages/example.dist-info/METADATA"),
            "Version: 2\n",
        );
        project.assert_changed();
        return;
    }
    let external = TempDir::new().unwrap();
    environment(external.path(), "lib/python3.13/site-packages");
    // Isolate environment mutation from Rust's parallel test harness.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "activated_environment_metadata_is_tracked",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("VIRTUAL_ENV", external.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
