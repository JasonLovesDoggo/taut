use crate::blocks::{BlockId, FileBlocks};
use crate::cache::ensure_cache_dir;
use crate::discovery::TestItem;
use serde::{Deserialize, Serialize};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use xxhash_rust::xxh64::Xxh64;

const DEPDB_FILE: &str = "depdb-v2.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TestId {
    pub file: PathBuf,
    pub function: String,
    pub class: Option<String>,
}

impl From<&TestItem> for TestId {
    fn from(item: &TestItem) -> Self {
        Self {
            file: canonical_path(&item.file),
            function: item.function.clone(),
            class: item.class.clone(),
        }
    }
}

impl std::fmt::Display for TestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.class {
            Some(class) => write!(f, "{}::{}::{}", self.file.display(), class, self.function),
            None => write!(f, "{}::{}", self.file.display(), self.function),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TestDependency {
    snapshot: Option<String>,
    last_run_passed: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DependencyDatabase {
    tests: HashMap<String, TestDependency>,
    /// Shared across tests because Python can cache an imported module before a
    /// later test starts tracing. Per-test line coverage cannot model that edge.
    coverage_files: BTreeSet<PathBuf>,
    #[serde(skip)]
    files: BTreeMap<PathBuf, String>,
    #[serde(skip)]
    blocks: HashMap<String, String>,
    #[serde(skip)]
    context: String,
    #[serde(skip)]
    incomplete: bool,
    #[serde(skip)]
    snapshot_cache: OnceCell<Option<String>>,
    #[serde(skip)]
    file_aliases: RefCell<HashMap<PathBuf, PathBuf>>,
}

impl DependencyDatabase {
    pub fn load() -> Self {
        ensure_cache_dir()
            .ok()
            .and_then(|dir| fs::File::open(dir.join(DEPDB_FILE)).ok())
            .and_then(|f| serde_json::from_reader(BufReader::new(f)).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Ok(dir) = ensure_cache_dir() else { return };
        // An interrupted write must not replace a usable database with a prefix.
        let temporary = dir.join(format!("depdb-{}.tmp", std::process::id()));
        if let Ok(file) = fs::File::create(&temporary) {
            let mut writer = BufWriter::new(file);
            if serde_json::to_writer(&mut writer, self).is_ok() && writer.flush().is_ok() {
                let _ = fs::rename(&temporary, dir.join(DEPDB_FILE));
            }
            let _ = fs::remove_file(temporary);
        }
    }

    pub(crate) fn coverage_files(&self) -> &BTreeSet<PathBuf> {
        &self.coverage_files
    }

    pub(crate) fn replace_files(&mut self, files: BTreeMap<PathBuf, String>, incomplete: bool) {
        self.snapshot_cache.take();
        self.file_aliases.get_mut().clear();
        self.files = files;
        self.blocks.clear();
        self.incomplete = incomplete;
    }

    pub(crate) fn set_context(&mut self, context: String) {
        self.snapshot_cache.take();
        self.context = context;
    }

    /// Replace a file's blocks, including removing identities that disappeared
    /// or moved. Selection uses the complete source bytes, not line coverage.
    pub fn update_blocks(&mut self, file_blocks: &FileBlocks) {
        self.snapshot_cache.take();
        let file = canonical_path(&file_blocks.file);
        self.blocks.retain(|key, _| {
            serde_json::from_str::<BlockId>(key).is_ok_and(|id| canonical_path(&id.file) != file)
        });
        for block in &file_blocks.blocks {
            if let Ok(key) = serde_json::to_string(&block.id) {
                self.blocks.insert(key, block.checksum.clone());
            }
        }
        self.files.insert(file, file_blocks.source_checksum.clone());
    }

    fn test_key(&self, test: &TestItem) -> String {
        // Canonicalization touches the filesystem; do it once per test file,
        // rather than once per test in a large module.
        let mut aliases = self.file_aliases.borrow_mut();
        let file = aliases
            .entry(test.file.clone())
            .or_insert_with(|| canonical_path(&test.file));
        serde_json::to_string(&TestId {
            file: file.clone(),
            function: test.function.clone(),
            class: test.class.clone(),
        })
        .expect("test identity is serializable")
    }

    fn snapshot(&self) -> Option<String> {
        self.snapshot_cache
            .get_or_init(|| self.compute_snapshot())
            .clone()
    }

    fn compute_snapshot(&self) -> Option<String> {
        if self.incomplete || self.files.is_empty() {
            return None;
        }
        let mut hash = Xxh64::new(0);
        hash.update(self.context.as_bytes());
        for (path, checksum) in &self.files {
            // Length prefixes keep path and checksum boundaries unambiguous.
            let bytes = path.as_os_str().as_encoded_bytes();
            hash.update(&(bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            hash.update(&(checksum.len() as u64).to_le_bytes());
            hash.update(checksum.as_bytes());
        }
        Some(format!("{:016x}", hash.digest()))
    }

    /// Coverage proves execution happened, but it is not a complete dependency
    /// graph. Every passed test belongs to the whole source snapshot.
    pub fn record_test_coverage(
        &mut self,
        test: &TestItem,
        coverage: &HashMap<PathBuf, Vec<usize>>,
        passed: bool,
        _block_index: &HashMap<PathBuf, FileBlocks>,
    ) {
        let id = TestId::from(test);
        let mut complete = false;
        for (path, lines) in coverage {
            let path = canonical_path(path);
            if path == id.file && !lines.is_empty() {
                complete = true;
            }
            self.coverage_files.insert(path.clone());
            if !self.files.contains_key(&path) {
                self.snapshot_cache.take();
                match fingerprint_file(&path) {
                    Ok(checksum) => {
                        self.files.insert(path, checksum);
                    }
                    Err(_) => {
                        self.incomplete = true;
                    }
                }
            }
        }
        let snapshot = complete.then(|| self.snapshot()).flatten();
        self.tests.insert(
            self.test_key(test),
            TestDependency {
                snapshot,
                last_run_passed: passed,
            },
        );
    }

    pub fn needs_run(&self, test: &TestItem) -> TestRunDecision {
        let key = self.test_key(test);
        let Some(dep) = self.tests.get(&key) else {
            return TestRunDecision::NeverRun;
        };
        if !dep.last_run_passed {
            return TestRunDecision::FailedLastTime;
        }
        match (&dep.snapshot, self.snapshot()) {
            (Some(expected), Some(current)) if *expected == current => TestRunDecision::CanSkip,
            (None, _) | (_, None) => TestRunDecision::CoverageIncomplete,
            _ => TestRunDecision::DependencyChanged,
        }
    }

    pub fn stats(&self) -> DepDbStats {
        let passed_tests = self.tests.values().filter(|t| t.last_run_passed).count();
        DepDbStats {
            total_blocks: self.blocks.len(),
            total_tests: self.tests.len(),
            passed_tests,
            failed_tests: self.tests.len() - passed_tests,
        }
    }
}

pub(crate) fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

pub(crate) fn fingerprint_file(path: &Path) -> std::io::Result<String> {
    fs::read(path).map(|bytes| format!("{:016x}", xxhash_rust::xxh64::xxh64(&bytes, 0)))
}

#[derive(Debug, Clone, Copy)]
pub enum TestRunDecision {
    CanSkip,
    NeverRun,
    FailedLastTime,
    DependencyChanged,
    DependencyDeleted,
    CoverageIncomplete,
}

impl TestRunDecision {
    pub fn should_run(&self) -> bool {
        !matches!(self, Self::CanSkip)
    }
    pub fn reason(&self) -> &'static str {
        match self {
            Self::CanSkip => "unchanged",
            Self::NeverRun => "new test",
            Self::FailedLastTime => "failed last run",
            Self::DependencyChanged => "source or execution context changed",
            Self::DependencyDeleted => "dependency deleted",
            Self::CoverageIncomplete => "incomplete execution coverage",
        }
    }
}

pub struct DepDbStats {
    pub total_blocks: usize,
    pub total_tests: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
}
