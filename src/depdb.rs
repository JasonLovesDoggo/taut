use crate::blocks::{BlockId, FileBlocks};
use crate::cache::ensure_cache_dir;
use crate::discovery::TestItem;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

const DEPDB_FILE: &str = "depdb.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TestId {
    pub file: PathBuf,
    pub function: String,
    pub class: Option<String>,
}

impl From<&TestItem> for TestId {
    fn from(item: &TestItem) -> Self {
        Self {
            file: item.file.clone(),
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
    /// Map: BlockId serialized key -> expected checksum
    dependencies: HashMap<String, String>,
    last_run_passed: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DependencyDatabase {
    /// All known blocks: serialized BlockId -> current checksum
    blocks: HashMap<String, String>,
    /// Test dependencies: serialized TestId -> dependency info
    tests: HashMap<String, TestDependency>,
}

impl DependencyDatabase {
    pub fn load() -> Self {
        let path = ensure_cache_dir()
            .map(|d| d.join(DEPDB_FILE))
            .unwrap_or_else(|_| PathBuf::from(DEPDB_FILE));

        if !path.exists() {
            return Self::default();
        }

        fs::File::open(&path)
            .ok()
            .and_then(|f| serde_json::from_reader(BufReader::new(f)).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = ensure_cache_dir()
            .map(|d| d.join(DEPDB_FILE))
            .unwrap_or_else(|_| PathBuf::from(DEPDB_FILE));

        if let Ok(f) = fs::File::create(&path) {
            let _ = serde_json::to_writer(BufWriter::new(f), self);
        }
    }

    fn block_key(block_id: &BlockId) -> String {
        serde_json::to_string(block_id).unwrap_or_default()
    }

    fn test_key(test_id: &TestId) -> String {
        serde_json::to_string(test_id).unwrap_or_default()
    }

    /// Update blocks from file parsing
    pub fn update_blocks(&mut self, file_blocks: &FileBlocks) {
        for block in &file_blocks.blocks {
            let key = Self::block_key(&block.id);
            self.blocks.insert(key, block.checksum.clone());
        }
    }

    /// Record test coverage after a test run
    pub fn record_test_coverage(
        &mut self,
        test: &TestItem,
        coverage: &HashMap<PathBuf, Vec<usize>>,
        passed: bool,
        block_index: &HashMap<PathBuf, FileBlocks>,
    ) {
        let test_id = TestId::from(test);
        let test_key = Self::test_key(&test_id);
        let mut dependencies = HashMap::new();

        // Map coverage lines to blocks
        for (file, lines) in coverage {
            if let Some(file_blocks) = block_index.get(file) {
                for &line in lines {
                    if let Some(block) = file_blocks.get_block_for_line(line) {
                        let block_key = Self::block_key(&block.id);
                        dependencies.insert(block_key, block.checksum.clone());
                    }
                }
            }
        }

        self.tests.insert(
            test_key,
            TestDependency {
                dependencies,
                last_run_passed: passed,
            },
        );
    }

    /// Check if a test needs to run based on changed blocks
    pub fn needs_run(&self, test: &TestItem) -> TestRunDecision {
        let test_id = TestId::from(test);
        let test_key = Self::test_key(&test_id);

        let Some(dep) = self.tests.get(&test_key) else {
            return TestRunDecision::NeverRun;
        };

        if !dep.last_run_passed {
            return TestRunDecision::FailedLastTime;
        }

        // Check if any dependencies changed
        for (block_key, expected_checksum) in &dep.dependencies {
            match self.blocks.get(block_key) {
                Some(current_checksum) => {
                    if current_checksum != expected_checksum {
                        return TestRunDecision::DependencyChanged;
                    }
                }
                None => {
                    return TestRunDecision::DependencyDeleted;
                }
            }
        }

        TestRunDecision::CanSkip
    }

    /// Get statistics
    pub fn stats(&self) -> DepDbStats {
        let passed_tests = self.tests.values().filter(|t| t.last_run_passed).count();
        let failed_tests = self.tests.len() - passed_tests;

        DepDbStats {
            total_blocks: self.blocks.len(),
            total_tests: self.tests.len(),
            passed_tests,
            failed_tests,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TestRunDecision {
    CanSkip,
    NeverRun,
    FailedLastTime,
    DependencyChanged,
    DependencyDeleted,
}

impl TestRunDecision {
    pub fn should_run(&self) -> bool {
        !matches!(self, TestRunDecision::CanSkip)
    }

    pub fn reason(&self) -> &'static str {
        match self {
            TestRunDecision::CanSkip => "unchanged",
            TestRunDecision::NeverRun => "new test",
            TestRunDecision::FailedLastTime => "failed last run",
            TestRunDecision::DependencyChanged => "dependency changed",
            TestRunDecision::DependencyDeleted => "dependency deleted",
        }
    }
}

pub struct DepDbStats {
    pub total_blocks: usize,
    pub total_tests: usize,
    pub passed_tests: usize,
    pub failed_tests: usize,
}
