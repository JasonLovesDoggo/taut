use crate::blocks::FileBlocks;
use crate::depdb::{DependencyDatabase, TestRunDecision, canonical_path, fingerprint_file};
use crate::discovery::TestItem;
use crate::runner::TestResult;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct TestSelection {
    pub to_run: Vec<(TestItem, TestRunDecision)>,
    pub to_skip: Vec<(TestItem, String)>,
}

impl TestSelection {
    pub fn run_count(&self) -> usize {
        self.to_run.len()
    }

    pub fn skip_count(&self) -> usize {
        self.to_skip.len()
    }
}

pub struct TestSelector {
    depdb: DependencyDatabase,
    block_index: HashMap<PathBuf, FileBlocks>,
    python_environment: Option<PathBuf>,
}

impl TestSelector {
    pub fn new() -> Self {
        let mut selector = Self {
            depdb: DependencyDatabase::load(),
            block_index: HashMap::new(),
            python_environment: None,
        };
        selector.set_execution_context("");
        selector
    }

    /// Bind cached results to the interpreter and execution options chosen by
    /// the caller. Only a digest is persisted; environment values stay private.
    pub fn set_execution_context(&mut self, context: &str) {
        let mut environment: Vec<_> = std::env::vars_os().collect();
        environment.sort();
        let mut hash = xxhash_rust::xxh64::Xxh64::new(0);
        hash.update(env!("CARGO_PKG_VERSION").as_bytes());
        hash.update(context.as_bytes());
        for (key, value) in environment {
            for component in [key, value] {
                let bytes = component.as_encoded_bytes();
                hash.update(&(bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
        self.depdb.set_context(format!("{:016x}", hash.digest()));
    }

    /// Track installed-package metadata for the selected virtual environment.
    /// Preserve the interpreter's original path: bin/python is often a symlink
    /// to a system interpreter outside the virtual environment.
    pub fn set_python_environment(&mut self, interpreter: &Path) {
        self.python_environment = interpreter
            .parent()
            .and_then(Path::parent)
            .filter(|root| root.join("pyvenv.cfg").is_file())
            .map(canonical_path);
    }

    /// Snapshot project sources, including application modules outside the test
    /// directory. Reading source bytes is cheaper and safer than AST-based
    /// dependency inference. Any addition, removal or edit reruns the suite.
    pub fn index_files(&mut self, paths: &[PathBuf]) {
        let roots: BTreeSet<_> = paths
            .iter()
            .map(|path| crate::project::root(path))
            .collect();
        let environments: BTreeSet<_> = roots
            .iter()
            .flat_map(|root| [root.join(".venv"), root.join("venv")])
            .chain(std::env::var_os("VIRTUAL_ENV").map(PathBuf::from))
            .chain(self.python_environment.clone())
            .map(|path| canonical_path(&path))
            .collect();
        let mut files = BTreeMap::new();
        let mut incomplete = roots.is_empty();
        for root in &roots {
            for entry in WalkDir::new(root)
                .follow_links(true)
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || !entry.file_type().is_dir()
                        || (!excluded_directory(entry.file_name())
                            && !environments.contains(entry.path()))
                })
            {
                match entry {
                    Ok(entry) if entry.file_type().is_file() && snapshot_file(entry.path()) => {
                        let path = canonical_path(entry.path());
                        match fingerprint_file(&path) {
                            Ok(checksum) => {
                                files.insert(path, checksum);
                            }
                            Err(_) => incomplete = true,
                        }
                    }
                    Ok(_) => {}
                    Err(_) => incomplete = true,
                }
            }
        }
        for environment in environments {
            snapshot_environment(&environment, &mut files, &mut incomplete);
        }
        // Imported helpers outside the root remain shared dependencies, even
        // when later tests reuse Python's module cache and emit no import lines.
        for path in self.depdb.coverage_files() {
            if files.contains_key(path) {
                continue;
            }
            match fingerprint_file(path) {
                Ok(checksum) => {
                    files.insert(path.clone(), checksum);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    files.insert(path.clone(), "<deleted>".to_string());
                }
                Err(_) => incomplete = true,
            }
        }
        self.block_index.clear();
        self.depdb.replace_files(files, incomplete);
    }

    /// Select which tests need to run based on dependency changes.
    /// Tests are sorted with failed tests first (fail-first strategy).
    pub fn select_tests(&self, all_tests: &[TestItem]) -> TestSelection {
        let mut to_run = Vec::new();
        let mut to_skip = Vec::new();

        for test in all_tests {
            let decision = self.depdb.needs_run(test);
            if decision.should_run() {
                to_run.push((test.clone(), decision));
            } else {
                to_skip.push((test.clone(), decision.reason().to_string()));
            }
        }

        // Sort tests with failed tests first (fail-first strategy)
        // This gives faster feedback on known failing tests
        to_run.sort_by(|(_, decision_a), (_, decision_b)| {
            use std::cmp::Ordering;
            let a_failed = matches!(decision_a, TestRunDecision::FailedLastTime);
            let b_failed = matches!(decision_b, TestRunDecision::FailedLastTime);
            match (a_failed, b_failed) {
                (true, false) => Ordering::Less,    // a failed, b didn't -> a first
                (false, true) => Ordering::Greater, // b failed, a didn't -> b first
                _ => Ordering::Equal,               // both same status -> keep order
            }
        });

        TestSelection { to_run, to_skip }
    }

    /// Record test result with coverage data
    pub fn record_result(&mut self, result: &TestResult) {
        if result.skipped {
            return;
        }
        if let Some(ref coverage) = result.coverage {
            self.depdb.record_test_coverage(
                &result.item,
                &coverage.files,
                result.passed,
                &self.block_index,
            );
        } else if !result.skipped {
            // Test ran without coverage - record empty dependency set
            self.depdb.record_test_coverage(
                &result.item,
                &HashMap::new(),
                result.passed,
                &self.block_index,
            );
        }
    }

    /// Save the dependency database
    pub fn save(&self) {
        self.depdb.save();
    }

    /// Get database statistics
    pub fn stats(&self) -> crate::depdb::DepDbStats {
        self.depdb.stats()
    }

    /// Get block index for coverage mapping
    pub fn block_index(&self) -> &HashMap<PathBuf, FileBlocks> {
        &self.block_index
    }
}

impl Default for TestSelector {
    fn default() -> Self {
        Self::new()
    }
}

fn excluded_directory(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            ".git"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".tox"
                | ".nox"
                | "node_modules"
                | "target"
                | ".taut"
        )
    )
}

fn snapshot_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("py" | "pyi")
    ) || matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            "pyproject.toml"
                | "setup.cfg"
                | "setup.py"
                | "pytest.ini"
                | "tox.ini"
                | "uv.lock"
                | "poetry.lock"
                | "Pipfile"
                | "Pipfile.lock"
                | "requirements.txt"
        )
    )
}

/// Inspect metadata only. Package installation, removal, and editable-install
/// changes invalidate the cache without walking thousands of package sources.
/// Manual edits to untraced installed code remain outside --changed's scope.
fn snapshot_environment(
    environment: &Path,
    files: &mut BTreeMap<PathBuf, String>,
    incomplete: &mut bool,
) {
    snapshot_optional_file(&environment.join("pyvenv.cfg"), files, incomplete);
    let mut package_directories =
        BTreeSet::from([canonical_path(&environment.join("Lib/site-packages"))]);
    for library in [environment.join("lib"), environment.join("lib64")] {
        for entry in directory_entries(&library, incomplete) {
            if entry.is_dir() {
                package_directories.insert(canonical_path(&entry.join("site-packages")));
            }
        }
    }
    for directory in package_directories {
        for path in directory_entries(&directory, incomplete) {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.ends_with(".pth") || name.ends_with(".egg-link") {
                snapshot_optional_file(&path, files, incomplete);
            } else if name.ends_with(".dist-info") || name.ends_with(".egg-info") {
                if path.is_dir() {
                    // Core distribution metadata lives at the top level.
                    // Nested license directories need no source traversal.
                    for metadata in directory_entries(&path, incomplete) {
                        if !metadata.is_dir() {
                            snapshot_optional_file(&metadata, files, incomplete);
                        }
                    }
                } else {
                    // Legacy egg-info can be a single metadata file.
                    snapshot_optional_file(&path, files, incomplete);
                }
            }
        }
    }
}

fn directory_entries(directory: &Path, incomplete: &mut bool) -> Vec<PathBuf> {
    match fs::read_dir(directory) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry.path()),
                Err(_) => {
                    *incomplete = true;
                    None
                }
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            *incomplete = true;
            Vec::new()
        }
    }
}

fn snapshot_optional_file(
    path: &Path,
    files: &mut BTreeMap<PathBuf, String>,
    incomplete: &mut bool,
) {
    // The environment and package roots are already absolute. Retain metadata
    // symlink paths so retargeting one is checked on the next snapshot, while
    // avoiding one expensive realpath call per installed metadata file.
    let path = path.to_path_buf();
    if files.contains_key(&path) {
        return;
    }
    match fingerprint_file(&path) {
        Ok(checksum) => {
            files.insert(path, checksum);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => *incomplete = true,
    }
}
