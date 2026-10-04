"""Guard against accidentally publishing successful timings for incomplete runs."""

from collections import Counter
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from compare_execution import BenchmarkError, make_project, validate_sample


class BenchmarkValidationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.audit = self.root / "audit"
        self.audit.mkdir()
        self.expected = ["test_part_000.py::test_00000", "test_part_001.py::TestWorkload::test_00001"]

    def witness(self, tests, prefix=None):
        (self.audit / "1.json").write_text(json.dumps({"tests": tests, "python": sys.executable,
                                                      "prefix": prefix or sys.prefix}))

    def result(self, stdout):
        return subprocess.CompletedProcess(["fake-runner"], 0, stdout, "")

    def test_exact_completion_is_required(self):
        self.witness(self.expected)
        validated = validate_sample(self.result("2 passed in 0.01s"), self.expected, self.audit, False, "pytest")
        self.assertEqual(validated["completed_count"], 2)
        for incomplete in ([], self.expected[:1], [self.expected[0], self.expected[0]]):
            with self.subTest(incomplete=incomplete):
                self.witness(incomplete)
                with self.assertRaises(BenchmarkError):
                    validate_sample(self.result("2 passed in 0.01s"), self.expected, self.audit, False, "taut")

    def test_nonzero_exit_is_rejected_even_with_passing_output(self):
        self.witness(self.expected)
        result = self.result("2 passed")
        result.returncode = 1
        with self.assertRaises(BenchmarkError):
            validate_sample(result, self.expected, self.audit, False, "taut")

    def test_false_success_and_cache_skips_are_rejected(self):
        self.witness(self.expected)
        for stdout in ("", "1 passed", "2 passed, 1 skipped", "2 passed, 1 failed", "No tests found"):
            with self.subTest(stdout=stdout), self.assertRaises(BenchmarkError):
                validate_sample(self.result(stdout), self.expected, self.audit, False, "taut")

    def test_different_python_environment_is_rejected(self):
        self.witness(self.expected, prefix=str(self.root / "wrong-venv"))
        with self.assertRaisesRegex(BenchmarkError, "different Python environment"):
            validate_sample(self.result("2 passed"), self.expected, self.audit, False, "taut")

    def test_collection_checks_exact_ids_and_no_executed_bodies(self):
        output = "\n".join(self.expected) + "\n2 tests collected in 0.01s"
        validated = validate_sample(self.result(output), self.expected, self.audit, True, "pytest")
        self.assertEqual(validated["completed_count"], 0)
        with self.assertRaises(BenchmarkError):
            validate_sample(self.result(output.replace(self.expected[1], self.expected[0])),
                            self.expected, self.audit, True, "pytest")
        self.witness(self.expected)
        with self.assertRaises(BenchmarkError):
            validate_sample(self.result(output), self.expected, self.audit, True, "pytest")

    def test_generates_exact_odd_counts(self):
        project = self.root / "generated"
        expected = make_project(project, "noop", 7, 3, .001, 10)
        self.assertEqual(len(expected), 7)
        self.assertEqual(len(set(expected)), 7)
        actual = []
        for path in project.glob("test_*.py"):
            for line in path.read_text().splitlines():
                if "completed(" in line:
                    actual.append(line.strip().removeprefix("completed(").removesuffix(")").strip("'"))
        self.assertEqual(Counter(actual), Counter(expected))


if __name__ == "__main__":
    unittest.main()
