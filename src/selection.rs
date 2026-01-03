use crate::blocks::FileBlocks;
use crate::depdb::{DependencyDatabase, TestRunDecision};
use crate::discovery::TestItem;
use crate::runner::TestResult;
use std::collections::HashMap;
use std::path::PathBuf;
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
}

impl TestSelector {
    pub fn new() -> Self {
        Self {
            depdb: DependencyDatabase::load(),
            block_index: HashMap::new(),
        }
    }

    /// Index all Python files in given paths
    pub fn index_files(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if path.is_file() && path.extension().is_some_and(|e| e == "py") {
                self.index_single_file(path);
            } else if path.is_dir() {
                for entry in WalkDir::new(path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_type().is_file()
                            && e.path().extension().is_some_and(|ext| ext == "py")
                    })
                {
                    self.index_single_file(entry.path());
                }
            }
        }
    }

    fn index_single_file(&mut self, path: &std::path::Path) {
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Ok(file_blocks) = FileBlocks::from_file(&abs_path) {
            self.depdb.update_blocks(&file_blocks);
            self.block_index.insert(abs_path, file_blocks);
        }
    }

    /// Select which tests need to run based on dependency changes
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

        TestSelection { to_run, to_skip }
    }

    /// Record test result with coverage data
    pub fn record_result(&mut self, result: &TestResult) {
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
