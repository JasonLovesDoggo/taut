"""Exercise the installed wheel in a clean project, with no optional dependencies."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

import taut
from taut import fixture, parametrize


heavy_imports = {"taut._taut", "typing", "inspect", "asyncio", "pytest"} & sys.modules.keys()
assert not heavy_imports, f"Public helpers eagerly imported {sorted(heavy_imports)}"
assert {"fixture", "parametrize"} <= set(taut.__all__)
assert callable(fixture) and callable(parametrize)


@taut.mark(group="packaging")
@taut.parallel
@taut.skip("example")
def decorated():
    pass


assert decorated._taut_markers == {"group": "packaging"}
assert decorated._taut_parallel
assert decorated._taut_skip_reason == "example"

binary = shutil.which("taut")
assert binary, "The wheel must install a taut executable"
with open(binary, "rb") as executable:
    assert executable.read(2) != b"#!", "taut must start as a native binary"

environment = os.environ.copy()
# Neither a developer's environment nor the checkout may complete the wheel.
# A restricted PATH also proves native executable-sibling interpreter lookup.
for name in ("PYTHONPATH", "TAUT_PYTHON", "VIRTUAL_ENV"):
    environment.pop(name, None)
environment["PATH"] = os.defpath

CONFTEST = '''
import asyncio
import os
from pathlib import Path
import uuid
from taut import fixture


def record(audit, event):
    with audit["events"].open("a", encoding="utf-8") as stream:
        stream.write(event + "\\n")


@fixture(name="shared_value")
def seed():
    return 7


@fixture(autouse=True)
def audit(tmp_path):
    directory = Path(os.environ["TAUT_SMOKE_AUDIT"])
    events = directory / (uuid.uuid4().hex + ".events")
    value = {"events": events, "temporary": tmp_path}
    previous = os.environ.get("TAUT_SMOKE_PATCH")
    record(value, "autouse setup")
    try:
        yield value
    finally:
        assert os.environ.get("TAUT_SMOKE_PATCH") == previous
        record(value, "autouse teardown")
        events.with_suffix(".temporary").write_text(str(tmp_path), encoding="utf-8")


@fixture
def sync_resource(audit, shared_value):
    record(audit, "sync setup")
    try:
        yield shared_value
    finally:
        record(audit, "sync teardown")


@fixture
async def async_resource(audit, shared_value):
    record(audit, "async setup")
    await asyncio.sleep(0)
    try:
        yield shared_value
    finally:
        await asyncio.sleep(0)
        record(audit, "async teardown")
'''

PASSING = f'''
import asyncio
import os
from pathlib import Path
import sys
import unittest
from taut import parallel, parametrize

EXPECTED_PREFIX = {sys.prefix!r}


def test_interpreter():
    assert sys.prefix == EXPECTED_PREFIX


@parallel
async def test_async(async_resource):
    await asyncio.sleep(0)
    assert async_resource == 7
    assert sys.prefix == EXPECTED_PREFIX


def test_fixture(sync_resource, audit, tmp_path, monkeypatch):
    monkeypatch.setenv("TAUT_SMOKE_PATCH", "patched")
    assert os.environ["TAUT_SMOKE_PATCH"] == "patched"
    assert sync_resource == 7
    assert tmp_path == audit["temporary"]
    assert tmp_path.is_dir()


@parametrize("value, expected", [(2, 4), (3, 6)], ids=["even", "odd"])
def test_double(value, expected, shared_value):
    assert value * 2 == expected
    assert shared_value == 7


class TestLifecycle(unittest.TestCase):
    def record(self, event):
        path = Path(os.environ["TAUT_SMOKE_AUDIT"]) / "unittest.events"
        with path.open("a", encoding="utf-8") as stream:
            stream.write(event + "\\n")

    def setUp(self):
        self.ready = True
        self.record("setup")
        self.addCleanup(self.record, "cleanup")

    def tearDown(self):
        self.record("teardown")

    def test_lifecycle(self):
        self.assertTrue(self.ready)
        self.record("test")
'''

FAILING = '''
from taut import parametrize


@parametrize("value", [1, 2], ids=["pass", "fail"])
def test_case(value, sync_resource):
    assert sync_resource == 7
    print(f"case: {value}")
    assert value == 1, "intentional parameter failure"
'''

PASSING_IDS = {
    "test_installed.py::test_interpreter",
    "test_installed.py::test_async",
    "test_installed.py::test_fixture",
    "test_installed.py::test_double[even]",
    "test_installed.py::test_double[odd]",
    "test_installed.py::TestLifecycle::test_lifecycle",
}
FAILURE_IDS = {"test_failure.py::test_case[pass]", "test_failure.py::test_case[fail]"}


def check_cleanup(audit, count, *, sync=0, asynchronous=0, lifecycle=False):
    paths = [path for path in audit.glob("*.events") if path.name != "unittest.events"]
    assert len(paths) == count, f"Autouse fixture ran {len(paths)} times, expected {count}"
    events = [path.read_text(encoding="utf-8").splitlines() for path in paths]
    for path, lines in zip(paths, events):
        assert lines[0] == "autouse setup" and lines[-1] == "autouse teardown", lines
        temporary = Path(path.with_suffix(".temporary").read_text(encoding="utf-8"))
        assert not temporary.exists(), f"tmp_path leaked: {temporary}"
    for kind, expected in (("sync", sync), ("async", asynchronous)):
        matching = [lines for lines in events if f"{kind} setup" in lines]
        assert len(matching) == expected, (kind, matching)
        for lines in matching:
            assert lines == ["autouse setup", f"{kind} setup", f"{kind} teardown", "autouse teardown"], lines
    if lifecycle:
        assert (audit / "unittest.events").read_text(encoding="utf-8").splitlines() == [
            "setup", "test", "teardown", "cleanup"
        ]


def run(command, root, isolation, filename, ids, *, failures=0, sync=0, asynchronous=0, lifecycle=False):
    with tempfile.TemporaryDirectory(prefix="audit-", dir=root) as directory:
        audit = Path(directory)
        result = subprocess.run(
            [*command, "--json", "--no-cache", "-j", "2", "--isolation", isolation, filename],
            cwd=root,
            env={**environment, "TAUT_SMOKE_AUDIT": str(audit)},
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=30,
        )
        assert result.returncode == (1 if failures else 0), (result.returncode, result.stdout, result.stderr)
        report = json.loads(result.stdout)
        summary = report["summary"]
        assert summary["collected"] == summary["executed"] == len(ids), summary
        assert summary["passed"] == len(ids) - failures and summary["failed"] == failures, summary
        assert summary["skipped"] == summary["unchanged"] == summary["not_run"] == 0, summary
        assert len(report["tests"]) == len(ids), report
        assert {test["id"] for test in report["tests"]} == ids, report
        for test in report["tests"]:
            if test["id"].endswith("[fail]"):
                assert test["status"] == "failed", test
                assert "intentional parameter failure" in test["error"]["message"], test
                assert "case: 2" in test["stdout"], test
        check_cleanup(audit, len(ids), sync=sync, asynchronous=asynchronous, lifecycle=lifecycle)


with tempfile.TemporaryDirectory(prefix="taut-wheel-") as directory:
    root = Path(directory)
    # A project boundary keeps conftest discovery independent of its temp parent.
    (root / "pyproject.toml").write_text("[tool.taut]\n", encoding="utf-8")
    (root / "conftest.py").write_text(CONFTEST, encoding="utf-8")
    (root / "test_installed.py").write_text(PASSING, encoding="utf-8")
    (root / "test_failure.py").write_text(FAILING, encoding="utf-8")
    for command in ([binary], [sys.executable, "-m", "taut"]):
        subprocess.run([*command, "--help"], cwd=root, env=environment, check=True, timeout=30)
        for isolation in ("process-per-run", "process-per-test"):
            run(command, root, isolation, "test_installed.py", PASSING_IDS,
                sync=1, asynchronous=1, lifecycle=True)
            run(command, root, isolation, "test_failure.py", FAILURE_IDS, failures=1, sync=2)
        run(command, root, "process-per-run", "test_failure.py::test_case[fail]",
            {"test_failure.py::test_case[fail]"}, failures=1, sync=1)

print("Installed wheel smoke tests passed")
