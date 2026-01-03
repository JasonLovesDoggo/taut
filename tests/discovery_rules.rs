use std::fs;
use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[test]
fn discover_files_with_test_prefixes() -> Result<()> {
    let tmp = TempDir::new()?;

    write_file(
        &tmp.path().join("test_alpha.py"),
        "def test_ok():\n    assert True\n",
    )?;
    write_file(
        &tmp.path().join("_test_beta.py"),
        "def test_ok():\n    assert True\n",
    )?;
    write_file(
        &tmp.path().join("not_a_test.py"),
        "def test_ok():\n    assert True\n",
    )?;

    let mut files = taut::discovery::find_test_files(&[tmp.path().to_path_buf()])?;
    files.sort();

    let rel: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(
        rel,
        vec!["_test_beta.py".to_string(), "test_alpha.py".to_string()]
    );
    Ok(())
}

#[test]
fn discover_function_names_test_and_test() -> Result<()> {
    let tmp = TempDir::new()?;
    let file = tmp.path().join("test_names.py");

    write_file(
        &file,
        r#"

def test_one():
    assert True

def _test_two():
    assert True

def not_a_test():
    assert True

class TestThing:
    def test_method(self):
        assert True

    def _test_private(self):
        assert True

class NotATest:
    def test_ignored(self):
        assert True
"#,
    )?;

    let items = taut::discovery::extract_tests_from_file(&file)?;
    let mut names: Vec<String> = items
        .iter()
        .map(|i| match &i.class {
            Some(cls) => format!("{}::{}", cls, i.function),
            None => i.function.clone(),
        })
        .collect();

    names.sort();

    assert_eq!(
        names,
        vec![
            "TestThing::_test_private".to_string(),
            "TestThing::test_method".to_string(),
            "_test_two".to_string(),
            "test_one".to_string(),
        ]
    );

    Ok(())
}
