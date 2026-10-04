use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use rustpython_parser::{Parse, ast};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

mod classes;

use crate::filter::TestFilter;
use crate::markers::{self, Marker};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestItem {
    pub file: PathBuf,
    pub function: String,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub line: usize,
    /// Markers attached to this test (@skip, @mark, @parallel)
    #[serde(default)]
    pub markers: Vec<Marker>,
}

impl TestItem {
    /// Returns a unique identifier for this test (e.g., "tests/test_example.py::TestMath::test_add")
    pub fn id(&self) -> String {
        let file = self.file.display();
        let mut id = match &self.class {
            Some(class) => format!("{}::{}::{}", file, class, self.function),
            None => format!("{}::{}", file, self.function),
        };
        if let Some(case) = crate::parametrize::case_id(self) {
            id.push_str(&format!("[{case}]"));
        }
        id
    }

    /// Keyword arguments for one statically collected parameter case.
    pub fn parameters(&self) -> Option<serde_json::Value> {
        crate::parametrize::parameters(self)
    }

    fn matches_function(&self, selector: &str) -> bool {
        self.function == selector
            || crate::parametrize::case_id(self)
                .is_some_and(|case| selector == format!("{}[{case}]", self.function))
    }

    /// Check if this test has the @skip marker.
    pub fn is_skipped(&self) -> bool {
        markers::is_skipped(&self.markers)
    }

    /// Get the skip reason if present.
    pub fn skip_reason(&self) -> Option<String> {
        markers::get_skip_reason(&self.markers)
    }

    /// Check if this test has the @parallel marker.
    pub fn is_parallel(&self) -> bool {
        markers::is_parallel(&self.markers)
    }

    /// Check if this test has @mark(slow=True).
    pub fn is_slow(&self) -> bool {
        markers::is_slow(&self.markers)
    }

    /// Get the group(s) from @mark(group="...").
    pub fn groups(&self) -> Vec<String> {
        markers::get_groups(&self.markers)
    }
}

/// Find Python tests under directories, or use explicitly named Python files.
///
/// Directory discovery recognizes `test_*.py`, `_test*.py`, and `*_test.py`.
/// Dependency, VCS, cache, and build directories are pruned; naming and pruning
/// rules never prevent an explicitly requested file or directory from being used.
/// Directory symlinks are not followed unless supplied as an explicit root.
/// Canonical identities deduplicate overlapping roots and file aliases while
/// retaining the first requested spelling for useful, relative test IDs.
pub fn find_test_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut seen_files = HashSet::new();
    let mut seen_dirs = HashSet::new();
    for path in paths {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Cannot collect {}", path.display()))?;
        if metadata.is_file() {
            if path.extension().is_none_or(|extension| extension != "py") {
                bail!(
                    "Cannot collect {}: expected a Python (.py) file",
                    path.display()
                );
            }
            add_file(path, &mut files, &mut seen_files)?;
        } else if metadata.is_dir() {
            let root_identity = canonical_path(path)?;
            let mut walker = WalkDir::new(path).sort_by_file_name().into_iter();
            while let Some(entry) = walker.next() {
                let entry = entry.with_context(|| format!("Cannot walk {}", path.display()))?;
                if entry.file_type().is_dir() {
                    if entry.depth() > 0 && is_ignored_directory(entry.file_name()) {
                        walker.skip_current_dir();
                        continue;
                    }
                    // WalkDir never follows descendant directory symlinks, so
                    // ordinary children share the canonical root's identity.
                    let identity = root_identity.join(entry.path().strip_prefix(path)?);
                    if !seen_dirs.insert(identity) {
                        walker.skip_current_dir();
                    }
                } else if is_test_file(entry.path()) {
                    // A test-file symlink is a valid alias. Directory symlinks
                    // remain untraversed to avoid escaping roots or cycles.
                    let is_file = if entry.file_type().is_symlink() {
                        std::fs::metadata(entry.path())
                            .with_context(|| format!("Cannot collect {}", entry.path().display()))?
                            .is_file()
                    } else {
                        entry.file_type().is_file()
                    };
                    if is_file {
                        let identity = if entry.file_type().is_symlink() {
                            canonical_path(entry.path())?
                        } else {
                            root_identity.join(entry.path().strip_prefix(path)?)
                        };
                        if seen_files.insert(identity) {
                            files.push(entry.path().to_path_buf());
                        }
                    }
                }
            }
        } else {
            bail!(
                "Cannot collect {}: expected a file or directory",
                path.display()
            );
        }
    }
    files.sort();
    Ok(files)
}

fn canonical_path(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("Cannot resolve {}", path.display()))
}

fn add_file(path: &Path, files: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) -> Result<()> {
    if seen.insert(canonical_path(path)?) {
        files.push(path.to_path_buf());
    }
    Ok(())
}

fn is_ignored_directory(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            ".git"
                | ".hg"
                | ".svn"
                | ".venv"
                | "venv"
                | "env"
                | "__pycache__"
                | ".pytest_cache"
                | ".mypy_cache"
                | ".ruff_cache"
                | ".tox"
                | ".nox"
                | "node_modules"
                | "site-packages"
                | "build"
                | "dist"
                | "target"
        )
    )
}

fn is_test_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".py")
        && (name.starts_with("test_") || name.starts_with("_test") || name.ends_with("_test.py"))
}

fn is_test_name(name: &str) -> bool {
    name.starts_with("test_") || name.starts_with("_test")
}

/// One scan of the source replaces a prefix scan for every discovered test.
struct LineIndex(Vec<usize>);

impl LineIndex {
    fn new(source: &str) -> Self {
        Self(
            source
                .bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
                .collect(),
        )
    }

    fn line(&self, offset: usize) -> usize {
        self.0.partition_point(|&newline| newline < offset) + 1
    }
}

fn test_function(stmt: &ast::Stmt) -> Option<(&str, usize, &[ast::Expr])> {
    match stmt {
        ast::Stmt::FunctionDef(func) if is_test_name(func.name.as_str()) => Some((
            func.name.as_str(),
            func.range.start().into(),
            &func.decorator_list,
        )),
        ast::Stmt::AsyncFunctionDef(func) if is_test_name(func.name.as_str()) => Some((
            func.name.as_str(),
            func.range.start().into(),
            &func.decorator_list,
        )),
        _ => None,
    }
}

fn make_item(
    path: &Path,
    stmt: &ast::Stmt,
    class: Option<&str>,
    inherited: &[ast::Expr],
    lines: &LineIndex,
    items: &mut Vec<TestItem>,
) -> Result<bool> {
    let Some((name, offset, decorators)) = test_function(stmt) else {
        return Ok(false);
    };
    let mut markers = markers::extract_markers(decorators);
    markers::inherit_markers(&mut markers, &markers::extract_class_markers(inherited));
    let item = TestItem {
        file: path.to_path_buf(),
        function: name.to_owned(),
        class: class.map(str::to_owned),
        line: lines.line(offset),
        markers,
    };
    crate::parametrize::expand_into(item, inherited.iter().chain(decorators), items).with_context(
        || {
            format!(
                "Cannot parametrize {}:{} ({name})",
                path.display(),
                lines.line(offset)
            )
        },
    )?;
    Ok(true)
}

// Static collection cannot decide which runtime branch defines a test. Report
// detectable unsupported definitions instead of silently omitting tests.
fn nested_test_offset(stmt: &ast::Stmt) -> Option<usize> {
    match stmt {
        ast::Stmt::FunctionDef(function)
            if is_test_name(function.name.as_str())
                || function.name.as_str().starts_with("test")
                || function.name.as_str() == "runTest" =>
        {
            Some(function.range.start().into())
        }
        ast::Stmt::AsyncFunctionDef(function)
            if is_test_name(function.name.as_str())
                || function.name.as_str().starts_with("test")
                || function.name.as_str() == "runTest" =>
        {
            Some(function.range.start().into())
        }
        ast::Stmt::ClassDef(class) => {
            if class.name.as_str().starts_with("Test") {
                Some(class.range.start().into())
            } else {
                class.body.iter().find_map(nested_test_offset)
            }
        }
        _ => compound_test_offset(stmt),
    }
}

fn suite_test_offset(suite: &[ast::Stmt]) -> Option<usize> {
    suite.iter().find_map(nested_test_offset)
}

fn compound_test_offset(stmt: &ast::Stmt) -> Option<usize> {
    match stmt {
        ast::Stmt::If(node) => {
            suite_test_offset(&node.body).or_else(|| suite_test_offset(&node.orelse))
        }
        ast::Stmt::For(node) => {
            suite_test_offset(&node.body).or_else(|| suite_test_offset(&node.orelse))
        }
        ast::Stmt::AsyncFor(node) => {
            suite_test_offset(&node.body).or_else(|| suite_test_offset(&node.orelse))
        }
        ast::Stmt::While(node) => {
            suite_test_offset(&node.body).or_else(|| suite_test_offset(&node.orelse))
        }
        ast::Stmt::With(node) => suite_test_offset(&node.body),
        ast::Stmt::AsyncWith(node) => suite_test_offset(&node.body),
        ast::Stmt::Match(node) => node
            .cases
            .iter()
            .find_map(|case| suite_test_offset(&case.body)),
        ast::Stmt::Try(node) => suite_test_offset(&node.body)
            .or_else(|| suite_test_offset(&node.orelse))
            .or_else(|| suite_test_offset(&node.finalbody))
            .or_else(|| {
                node.handlers.iter().find_map(|handler| {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    suite_test_offset(&handler.body)
                })
            }),
        ast::Stmt::TryStar(node) => suite_test_offset(&node.body)
            .or_else(|| suite_test_offset(&node.orelse))
            .or_else(|| suite_test_offset(&node.finalbody))
            .or_else(|| {
                node.handlers.iter().find_map(|handler| {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    suite_test_offset(&handler.body)
                })
            }),
        _ => None,
    }
}

fn reject_compound_tests(path: &Path, stmt: &ast::Stmt, lines: &LineIndex) -> Result<()> {
    if let Some(offset) = compound_test_offset(stmt) {
        bail!(
            "Cannot collect {}:{}: test definitions inside conditional or compound statements are unsupported; define tests directly in the module or test class",
            path.display(),
            lines.line(offset)
        );
    }
    Ok(())
}

/// Parse a Python file without importing it and extract statically defined tests.
pub fn extract_tests_from_file(path: &Path) -> Result<Vec<TestItem>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let suite = ast::Suite::parse(&source, &path.to_string_lossy())
        .map_err(|error| anyhow::anyhow!("Parse error in {}: {}", path.display(), error))?;
    let lines = LineIndex::new(&source);
    let classes = classes::Classes::needed(&suite).then(|| classes::Classes::new(&suite));
    if let Some(classes) = &classes {
        classes.reject_conditional_classes(path, &lines)?;
    }
    let mut items = Vec::new();
    for stmt in &suite {
        if make_item(path, stmt, None, &[], &lines, &mut items)? {
            continue;
        } else if let ast::Stmt::ClassDef(class) = stmt {
            if let Some(classes) = &classes {
                items.extend(classes.collect(class, path, &lines)?);
            }
        } else {
            reject_compound_tests(path, stmt, &lines)?;
        }
    }
    if let Some(classes) = classes {
        items.extend(classes.collect_aliases(path, &lines)?);
    }
    Ok(items)
}

fn apply_filter(items: &mut Vec<TestItem>, pattern: Option<&str>) -> Result<()> {
    if let Some(pattern) = pattern.filter(|pattern| !pattern.is_empty()) {
        let filter = TestFilter::new(pattern)
            .with_context(|| format!("Invalid filter pattern '{pattern}'"))?;
        items.retain(|item| filter.matches(&item.id()));
    }
    Ok(())
}

/// Extract tests from files. Any collection failure aborts the operation, so a
/// passing result can never conceal an unreadable or syntactically broken file.
pub fn extract_tests(files: &[PathBuf], filter_pattern: Option<&str>) -> Result<Vec<TestItem>> {
    // Avoid starting the Rayon pool for the common single-file/edit loop.
    // Indexed parallel collection preserves deterministic file and source order.
    let mut items = if files.len() >= 128 {
        files
            .par_iter()
            .map(|file| extract_tests_from_file(file))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect()
    } else {
        let mut items = Vec::new();
        for file in files {
            items.extend(extract_tests_from_file(file)?);
        }
        items
    };
    apply_filter(&mut items, filter_pattern)?;
    Ok(items)
}

/// Strip a positional node selector for config lookup, indexing, and watching.
pub fn selection_path(path: &Path) -> PathBuf {
    path.to_str()
        .and_then(|path| path.split_once("::"))
        .map_or_else(|| path.to_path_buf(), |(file, _)| PathBuf::from(file))
}

struct NodeSelector {
    file: PathBuf,
    nodes: Vec<String>,
    display: String,
    matched: bool,
}

impl NodeSelector {
    fn matches(&self, item: &TestItem) -> bool {
        match self.nodes.as_slice() {
            [name] => item
                .class
                .as_ref()
                .map_or_else(|| item.matches_function(name), |class| class == name),
            [class, method] => item.class.as_ref() == Some(class) && item.matches_function(method),
            _ => false,
        }
    }
}

/// Collect directories, files, and exact positional node IDs in a single pass.
/// Selectors accept `file.py::test_name`, `file.py::TestClass`, or
/// `file.py::TestClass::test_method`; `-k` remains the separate glob filter.
pub fn collect_tests(paths: &[PathBuf], filter_pattern: Option<&str>) -> Result<Vec<TestItem>> {
    let mut roots = Vec::new();
    let mut selectors = Vec::new();
    for path in paths {
        if let Some((file, nodes)) = path.to_str().and_then(|path| path.split_once("::")) {
            let nodes: Vec<String> = nodes.split("::").map(str::to_owned).collect();
            if file.is_empty()
                || nodes.is_empty()
                || nodes.len() > 2
                || nodes.iter().any(String::is_empty)
            {
                bail!(
                    "Invalid test selector '{}': expected file.py::test_name or file.py::TestClass::test_method",
                    path.display()
                );
            }
            let file = PathBuf::from(file);
            if !std::fs::metadata(&file)
                .with_context(|| format!("Cannot collect {}", file.display()))?
                .is_file()
            {
                bail!(
                    "Invalid test selector '{}': selectors require a Python file",
                    path.display()
                );
            }
            selectors.push(NodeSelector {
                file: canonical_path(&file)?,
                nodes,
                display: path.display().to_string(),
                matched: false,
            });
        } else {
            roots.push(path.clone());
        }
    }
    let mut files = find_test_files(&roots)?;
    if selectors.is_empty() {
        return extract_tests(&files, filter_pattern);
    }
    let unrestricted: HashSet<PathBuf> = files
        .iter()
        .map(|file| canonical_path(file))
        .collect::<Result<_>>()?;
    let mut identities = unrestricted.clone();
    let mut selectors_by_file: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (index, selector) in selectors.iter().enumerate() {
        selectors_by_file
            .entry(selector.file.clone())
            .or_default()
            .push(index);
        let file = selection_path(Path::new(&selector.display));
        if file.extension().is_none_or(|extension| extension != "py") {
            bail!(
                "Cannot collect {}: expected a Python (.py) file",
                file.display()
            );
        }
        if identities.insert(selector.file.clone()) {
            files.push(file);
        }
    }
    files.sort();
    let mut items = Vec::new();
    for file in files {
        let identity = canonical_path(&file)?;
        let all = unrestricted.contains(&identity);
        let file_selectors = selectors_by_file
            .get(&identity)
            .map_or(&[][..], Vec::as_slice);
        for item in extract_tests_from_file(&file)? {
            let mut selected = all;
            for &index in file_selectors {
                let selector = &mut selectors[index];
                if selector.matches(&item) {
                    selector.matched = true;
                    selected = true;
                }
            }
            if selected {
                items.push(item);
            }
        }
    }
    for selector in selectors {
        if !selector.matched {
            bail!(
                "Test selector '{}' did not match any test",
                selector.display
            );
        }
    }
    apply_filter(&mut items, filter_pattern)?;
    Ok(items)
}
