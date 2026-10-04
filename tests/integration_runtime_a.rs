use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("pyproject.toml"), "[tool.taut]\n").unwrap();
    for (name, content) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    dir
}
fn run(dir: &Path, more: &[&str]) -> (i32, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_taut"))
        .args(["--json", "-j", "1", "--timeout", "3"])
        .args(more)
        .current_dir(dir)
        .env(
            "PYTHONPATH",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("python"),
        )
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let report=serde_json::from_slice(&output.stdout).unwrap_or_else(|_|serde_json::json!({"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)}));
    (output.status.code().unwrap(), report)
}
fn passed(report: &(i32, Value), count: usize) {
    assert_eq!(report.0, 0, "{}", report.1);
    assert_eq!(report.1["summary"]["passed"], count, "{}", report.1);
}

#[test]
fn conftest_autouse_dependencies_and_sync_fixture_cleanup_execute() {
    let dir = project(&[
        (
            "conftest.py",
            r#"from taut import fixture
@fixture(autouse=True)
def environment():
 print('autouse setup')
 yield
 print('autouse cleanup')
@fixture
def base():
 value=[]
 print('base setup')
 yield value
 print('base cleanup')
@fixture
def first(base): return base
@fixture
def second(base): return base
"#,
        ),
        (
            "tests/test_sync.py",
            r#"def test_values(first,second,tmp_path):
 assert first is second
 assert tmp_path.is_dir()
 print('body')
"#,
        ),
    ]);
    let report = run(dir.path(), &[]);
    passed(&report, 1);
    let stdout = report.1["tests"][0]["stdout"].as_str().unwrap();
    assert_eq!(
        stdout,
        "autouse setup\nbase setup\nbody\nbase cleanup\nautouse cleanup\n"
    );
}

#[test]
fn sync_fixture_and_test_can_both_call_asyncio_run() {
    let dir = project(&[(
        "test_sync.py",
        r#"import asyncio
from taut import fixture
async def value(): return 42
@fixture
def answer():
 yield asyncio.run(value())
 assert asyncio.run(value()) == 42
def test_answer(answer): assert answer == asyncio.run(value())
"#,
    )]);
    passed(&run(dir.path(), &[]), 1);
}

#[test]
fn async_fixture_lifecycle_uses_one_loop_and_context_per_parameter_case() {
    let dir = project(&[(
        "test_async.py",
        r#"import asyncio, contextvars
from taut import fixture, parametrize
current=contextvars.ContextVar('current')
finished=[]
@fixture
async def resource(value):
 token=current.set(value)
 loop=asyncio.get_running_loop()
 try:
  yield loop
 finally:
  assert current.get() == value
  assert asyncio.get_running_loop() is loop
  current.reset(token)
  finished.append(value)
@parametrize('value',[1,2,3,4])
async def test_values(value,resource):
 assert current.get() == value
 assert asyncio.get_running_loop() is resource
 await asyncio.sleep(0.01)
 assert current.get() == value
def test_finished(): assert sorted(finished) == [1,2,3,4]
"#,
    )]);
    passed(&run(dir.path(), &["--async-concurrency", "4"]), 5);
}

#[test]
fn fixtures_cleanup_on_setup_failure_and_preserve_body_and_teardown_failures() {
    let dir = project(&[(
        "test_failure.py",
        r#"from taut import fixture
@fixture
def first():
 yield 1
 print('first cleanup')
@fixture
def broken(first): raise ValueError('setup failed')
@fixture
def teardown():
 yield
 raise RuntimeError('teardown failed')
def test_setup(broken): raise AssertionError('never runs')
def test_body(teardown): raise ValueError('body failed')
"#,
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert_eq!(report.1["summary"]["failed"], 2);
    assert_eq!(report.1["tests"][0]["stdout"], "first cleanup\n");
    let error = report.1["tests"][1]["error"]["message"].as_str().unwrap();
    assert!(
        error.contains("body failed") && error.contains("teardown failed"),
        "{error}"
    );
}

#[test]
fn monkeypatch_dependency_runs_alone_inside_shared_async_worker() {
    let dir = project(&[(
        "test_monkey.py",
        r#"import asyncio, os
from taut import fixture, parametrize
@fixture
def environment(monkeypatch,value):
 monkeypatch.setenv('TAUT_CASE',str(value))
 yield
@parametrize('value',[1,2,3,4])
async def test_values(value,environment):
 await asyncio.sleep(0.01)
 assert os.environ['TAUT_CASE'] == str(value)
def test_restored(): assert 'TAUT_CASE' not in os.environ
"#,
    )]);
    passed(&run(dir.path(), &["--async-concurrency", "4"]), 5);
}

#[test]
fn direct_parameter_names_never_request_builtin_or_autouse_fixtures() {
    let dir = project(&[(
        "test_params.py",
        r#"from taut import parametrize
@parametrize('monkeypatch',[{'z':1,'a':[True,None,3]}], ids=['mapping'])
def test_mapping(monkeypatch):
 assert list(monkeypatch) == ['z','a']
 assert monkeypatch['a'] == [True,None,3]
@parametrize('left',[1,2],ids=['one','two'])
@parametrize('right',['a','b'],ids=['A','B'])
def test_product(left,right): assert left in [1,2] and right in ['a','b']
"#,
    )]);
    let report = run(dir.path(), &[]);
    passed(&report, 5);
    assert!(
        report.1["tests"][0]["id"]
            .as_str()
            .unwrap()
            .ends_with("[mapping]")
    );
    passed(
        &run(dir.path(), &["test_params.py::test_product[A-one]"]),
        1,
    );
}

#[test]
fn tuple_valued_parameter_is_rejected_before_invocation() {
    let dir = project(&[(
        "test_tuple.py",
        "from taut import parametrize\n@parametrize('value', [(1, 2)])\ndef test_value(value): assert isinstance(value, tuple)\n",
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 2, "{}", report.1);
    assert!(report.1.to_string().contains("tuple"));
}

#[test]
fn unittest_run_subtests_and_registered_cleanup_are_real() {
    let dir = project(&[(
        "test_unit.py",
        r#"import unittest
class Cases(unittest.TestCase):
 def run(self,result=None):
  print('run called')
  return super().run(result)
 def setUp(self): self.addCleanup(lambda: print('cleanup called'))
 def test_subtests(self):
  for value in [1,2,3]:
   with self.subTest(value=value): self.assertLess(value,2)
  print('all subtests ran')
 def test_skip(self): raise unittest.SkipTest('skip reason')
"#,
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert_eq!(report.1["summary"]["failed"], 1);
    assert_eq!(report.1["summary"]["skipped"], 1);
    let tests = report.1["tests"].as_array().unwrap();
    let failure = tests
        .iter()
        .find(|test| test["status"] == "failed")
        .unwrap();
    assert_eq!(
        failure["stdout"],
        "run called\nall subtests ran\ncleanup called\n"
    );
    assert!(
        failure["error"]["message"]
            .as_str()
            .unwrap()
            .contains("value=3")
    );
}

#[test]
fn unittest_setup_failure_executes_cleanup_and_unawaited_tests_fail() {
    let dir = project(&[(
        "test_unit.py",
        r#"import unittest
class Broken(unittest.TestCase):
 def setUp(self):
  self.addCleanup(lambda: print('setup cleanup'))
  raise ValueError('setup broke')
 def test_case(self): pass
class Invalid(unittest.TestCase):
 async def test_async(self): assert False
 def test_generator(self):
  assert False
  yield 1
"#,
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert_eq!(report.1["summary"]["failed"], 3);
    assert_eq!(report.1["tests"][0]["stdout"], "setup cleanup\n");
}

#[test]
fn isolated_asyncio_case_keeps_its_own_loop_for_fixtures_and_cleanup() {
    let dir = project(&[(
        "test_isolated.py",
        r#"import unittest,asyncio
from taut import fixture
@fixture
async def loop():
 current=asyncio.get_running_loop()
 yield current
 assert asyncio.get_running_loop() is current
 print('fixture cleanup')
class Isolated(unittest.IsolatedAsyncioTestCase):
 async def asyncSetUp(self):
  self.loop=asyncio.get_running_loop()
  async def cleanup():
   assert asyncio.get_running_loop() is self.loop
   print('async cleanup')
  self.addAsyncCleanup(cleanup)
 async def test_loop(self,loop): assert loop is self.loop
 def test_sync(self):
  async def answer(): return 42
  assert asyncio.run(answer()) == 42
"#,
    )]);
    let report = run(dir.path(), &["--async-concurrency", "4"]);
    passed(&report, 2);
    assert!(
        report.1["tests"][0]["stdout"]
            .as_str()
            .unwrap()
            .contains("fixture cleanup")
    );
}

#[test]
fn ordinary_test_keeps_asyncio_and_fixture_runtime_unloaded() {
    let dir = project(&[(
        "test_plain.py",
        "import sys\ndef test_plain():\n assert 'asyncio' not in sys.modules\n assert '_taut_fixtures' not in sys.modules\n",
    )]);
    passed(&run(dir.path(), &[]), 1);
}

#[test]
fn xunit_function_and_method_hooks_run_in_order() {
    let dir = project(&[(
        "test_hooks.py",
        r#"events=[]
def setup_function(function): events.append(function.__name__)
def teardown_function(function): events.clear()
def test_function(): assert events == ['test_function']
class TestMethod:
 def setup_method(self,method): self.value=method.__name__
 def teardown_method(self): print('method cleanup')
 def test_method(self): assert self.value=='test_method'
"#,
    )]);
    passed(&run(dir.path(), &[]), 2);
}

#[test]
fn unexpanded_aliased_parametrize_cannot_pass_only_the_default_case() {
    let dir = project(&[(
        "test_alias.py",
        "from taut import parametrize as cases\n@cases('value',[0,1])\ndef test_value(value=0): assert value==0\n",
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert!(report.1.to_string().contains("not fully expanded"));
}

#[test]
fn unittest_parameter_values_reach_the_real_bound_method() {
    let dir = project(&[(
        "test_params.py",
        "import unittest\nfrom taut import parametrize\nclass Case(unittest.TestCase):\n @parametrize('value',[0,1])\n def test_value(self,value=0): self.assertEqual(value,0)\n",
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert_eq!(report.1["summary"]["passed"], 1);
    assert_eq!(report.1["summary"]["failed"], 1);
}

#[test]
fn optimized_python_cannot_disable_failing_assertions() {
    let dir = project(&[("test_assert.py", "def test_assert(): assert False\n")]);
    let output = Command::new(env!("CARGO_BIN_EXE_taut"))
        .args(["--json", "-j", "1"])
        .current_dir(dir.path())
        .env("PYTHONOPTIMIZE", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("optimization disables test assertions")
    );
}

#[test]
fn partial_aliased_parameter_expansion_is_rejected() {
    let dir = project(&[(
        "test_alias.py",
        "from taut import parametrize, parametrize as cases\n@parametrize('x',[0])\n@cases('y',[0,1])\ndef test_value(x,y=0): assert y==0\n",
    )]);
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert!(report.1.to_string().contains("not fully expanded"));
}

#[test]
fn local_helper_source_wins_over_stale_timestamp_bytecode() {
    let dir = project(&[
        ("helper.py", "VALUE=1\n"),
        (
            "test_helper.py",
            "import helper\ndef test_value(): assert helper.VALUE == 1\n",
        ),
    ]);
    let python = taut::worker_pool::resolve_python(None);
    let status=Command::new(python).args(["-c","import os,py_compile; os.utime('helper.py',(1000000000,1000000000)); py_compile.compile('helper.py'); open('helper.py','w').write('VALUE=2\\n'); os.utime('helper.py',(1000000000,1000000000))"]).current_dir(dir.path()).status().unwrap();
    assert!(status.success());
    let report = run(dir.path(), &[]);
    assert_eq!(report.0, 1, "{}", report.1);
    assert_eq!(report.1["summary"]["failed"], 1);
}
