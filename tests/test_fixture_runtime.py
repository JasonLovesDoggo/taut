"""Contract tests for fixture lifetimes; run with python tests/test_fixture_runtime.py."""

import asyncio
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
import weakref


ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


public = load("_fixture_decorators", ROOT / "python/taut/fixtures.py")
runtime = load("_fixture_runtime", ROOT / "src/fixtures.py")
fixture = public.fixture
FixtureRegistry = runtime.FixtureRegistry
FixtureContext = runtime.FixtureContext
FixtureError = runtime.FixtureError


def registry(**functions):
    module = types.ModuleType("fixture_test")
    module.__dict__.update(functions)
    return FixtureRegistry.from_module(module)


class FixtureTests(unittest.IsolatedAsyncioTestCase):
    async def test_shared_dependencies_once_and_reverse_cleanup(self):
        events = []

        @fixture
        def database():
            events.append("database setup")
            yield []
            events.append("database cleanup")

        @fixture
        def first(database):
            events.append("first setup")
            yield database
            events.append("first cleanup")

        @fixture
        def second(database):
            events.append("second setup")
            yield database
            events.append("second cleanup")

        def test(first, second):
            self.assertIs(first, second)

        async with FixtureContext(registry(database=database, first=first, second=second)) as context:
            test(**await context.kwargs(test))
        self.assertEqual(events, ["database setup", "first setup", "second setup", "second cleanup", "first cleanup", "database cleanup"])

    async def test_cleanup_after_test_failure(self):
        events = []

        @fixture
        def resource():
            yield object()
            events.append("closed")

        with self.assertRaisesRegex(AssertionError, "test failed"):
            async with FixtureContext(registry(resource=resource)) as context:
                await context.resolve("resource")
                raise AssertionError("test failed")
        self.assertEqual(events, ["closed"])

    async def test_partial_setup_failure_cleans_dependencies(self):
        events = []

        @fixture
        def resource():
            yield object()
            events.append("closed")

        @fixture
        def broken(resource):
            raise ValueError("setup failed")

        with self.assertRaisesRegex(ValueError, "setup failed"):
            async with FixtureContext(registry(resource=resource, broken=broken)) as context:
                await context.resolve("broken")
        self.assertEqual(events, ["closed"])

    async def test_async_fixtures_share_loop_with_test_and_cleanup(self):
        events = []
        expected_loop = asyncio.get_running_loop()

        @fixture
        async def value():
            await asyncio.sleep(0)
            return 42

        @fixture
        async def resource(value):
            self.assertIs(asyncio.get_running_loop(), expected_loop)
            yield value
            await asyncio.sleep(0)
            self.assertIs(asyncio.get_running_loop(), expected_loop)
            events.append("closed")

        async def test(resource):
            self.assertEqual(resource, 42)

        async with FixtureContext(registry(value=value, resource=resource)) as context:
            await test(**await context.kwargs(test))
        self.assertEqual(events, ["closed"])

    async def test_cancellation_cleans_async_fixture(self):
        events = []
        ready = asyncio.Event()

        @fixture
        async def resource():
            try:
                yield object()
            finally:
                await asyncio.sleep(0)
                events.append("closed")

        async def run():
            async with FixtureContext(registry(resource=resource)) as context:
                await context.resolve("resource")
                ready.set()
                await asyncio.sleep(60)

        task = asyncio.create_task(run())
        await ready.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(events, ["closed"])

    async def test_autouse_runs_with_no_arguments_and_is_cached(self):
        events = []

        @fixture(autouse=True)
        def prepare():
            events.append("setup")
            yield 9
            events.append("cleanup")

        def test():
            pass

        definitions = registry(prepare=prepare)
        self.assertTrue(definitions.needs_context(test))
        async with FixtureContext(definitions) as context:
            self.assertEqual(await context.kwargs(test), {})
            self.assertEqual(await context.resolve("prepare"), 9)
        self.assertEqual(events, ["setup", "cleanup"])

    async def test_context_values_are_isolated(self):
        @fixture
        def value():
            return []

        definitions = registry(value=value)
        async with FixtureContext(definitions) as first, FixtureContext(definitions) as second:
            self.assertIsNot(await first.resolve("value"), await second.resolve("value"))

    async def test_missing_fixture_explains_dependency(self):
        @fixture
        def service(missing):
            return missing

        async with FixtureContext(registry(service=service)) as context:
            with self.assertRaisesRegex(FixtureError, "service -> missing.*available: monkeypatch, service, tmp_path"):
                await context.resolve("service")

    async def test_cycle_explains_chain(self):
        @fixture
        def a(b):
            return b

        @fixture
        def b(a):
            return a

        async with FixtureContext(registry(a=a, b=b)) as context:
            with self.assertRaisesRegex(FixtureError, "a -> b -> a"):
                await context.resolve("a")

    async def test_default_values_are_preserved(self):
        def test(value=12):
            return value

        definitions = registry()
        self.assertFalse(definitions.needs_context(test))
        async with FixtureContext(definitions) as context:
            self.assertEqual(test(**await context.kwargs(test)), 12)

    async def test_keyword_only_fixture(self):
        def test(*, tmp_path):
            self.assertTrue(tmp_path.is_dir())

        async with FixtureContext(registry()) as context:
            test(**await context.kwargs(test))

    def test_signature_cache_does_not_retain_test_instances(self):
        class Tests:
            def test_example(self, tmp_path):
                pass

        definitions = registry()
        instance = Tests()
        reference = weakref.ref(instance)
        self.assertEqual(definitions.dependencies(instance.test_example), ("tmp_path",))
        del instance
        self.assertIsNone(reference())

    async def test_tmp_path_removed_and_unique(self):
        definitions = registry()
        async with FixtureContext(definitions) as first, FixtureContext(definitions) as second:
            first_path = await first.resolve("tmp_path")
            second_path = await second.resolve("tmp_path")
            self.assertNotEqual(first_path, second_path)
            (first_path / "value").write_text("test")
        self.assertFalse(first_path.exists())
        self.assertFalse(second_path.exists())

    async def test_monkeypatch_undo_after_failure(self):
        key = "TAUT_FIXTURE_TEST_SENTINEL"
        old_value = os.environ.get(key)
        old_cwd = os.getcwd()
        mapping = {"key": 1}
        target = types.SimpleNamespace(value=1)
        with self.assertRaises(AssertionError):
            async with FixtureContext(registry()) as context:
                patch = await context.resolve("monkeypatch")
                path = await context.resolve("tmp_path")
                patch.setenv(key, "new")
                patch.setitem(mapping, "key", 2)
                patch.setattr(target, "value", 2)
                patch.chdir(path)
                raise AssertionError("test failed")
        self.assertEqual(os.environ.get(key), old_value)
        self.assertEqual(os.getcwd(), old_cwd)
        self.assertEqual(mapping, {"key": 1})
        self.assertEqual(target.value, 1)

    async def test_monkeypatch_dependency_rejected_before_setup_when_concurrent(self):
        events = []

        @fixture(autouse=True)
        def preliminary():
            events.append("setup")

        @fixture
        def service(monkeypatch):
            return monkeypatch

        def test(service):
            pass

        definitions = registry(service=service, preliminary=preliminary)
        self.assertTrue(definitions.requires_serial(test))
        async with FixtureContext(definitions, concurrent=True) as context:
            with self.assertRaisesRegex(FixtureError, "serial worker slot"):
                await context.kwargs(test)
        self.assertEqual(events, [])

    async def test_all_finalizers_run_when_one_fails(self):
        events = []

        @fixture
        def first():
            yield
            events.append("first")
            raise ValueError("first cleanup")

        @fixture
        def second(first):
            yield
            events.append("second")
            raise ValueError("second cleanup")

        with self.assertRaises(ExceptionGroup) as caught:
            async with FixtureContext(registry(first=first, second=second)) as context:
                await context.resolve("second")
        self.assertEqual(events, ["second", "first"])
        self.assertEqual([str(e) for e in caught.exception.exceptions], ["second cleanup", "first cleanup"])

    async def test_test_and_cleanup_errors_both_preserved(self):
        @fixture
        def broken():
            yield
            raise ValueError("cleanup failed")

        with self.assertRaises(ExceptionGroup) as caught:
            async with FixtureContext(registry(broken=broken)) as context:
                await context.resolve("broken")
                raise AssertionError("test failed")
        self.assertEqual([str(e) for e in caught.exception.exceptions], ["test failed", "cleanup failed"])

    async def test_bad_generator_contract(self):
        @fixture
        def empty():
            yield from ()

        @fixture
        def twice():
            yield 1
            yield 2

        async with FixtureContext(registry(empty=empty)) as context:
            with self.assertRaisesRegex(FixtureError, "did not yield"):
                await context.resolve("empty")
        with self.assertRaisesRegex(FixtureError, "yielded more than once"):
            async with FixtureContext(registry(twice=twice)) as context:
                await context.resolve("twice")

    async def test_conftest_ancestors_override_and_autouse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            nested = root / "tests"
            nested.mkdir()
            (root / "conftest.py").write_text("from _fixture_decorators import fixture\n@fixture\ndef number(): return 1\n")
            (nested / "conftest.py").write_text("from _fixture_decorators import fixture\n@fixture\ndef number(): return 2\n")
            test_file = nested / "test_example.py"
            test_file.write_text("from _fixture_decorators import fixture\n@fixture\ndef number(): return 3\n")
            module = load("fixture_conftest_override_test", test_file)
            async with FixtureContext(FixtureRegistry.from_module(module, test_file, root)) as context:
                self.assertEqual(await context.resolve("number"), 3)
            del module.number
            async with FixtureContext(FixtureRegistry.from_module(module, test_file, root)) as context:
                self.assertEqual(await context.resolve("number"), 2)

    async def test_pytest_fixture_if_installed(self):
        try:
            import pytest
        except ImportError:
            self.skipTest("pytest is optional")

        @pytest.fixture(name="aliased")
        def value():
            yield 42

        async with FixtureContext(registry(value=value)) as context:
            self.assertEqual(await context.resolve("aliased"), 42)
        with self.assertRaisesRegex(FixtureError, "only function scope"):
            registry(value=pytest.fixture(scope="session")(lambda: 1))
        with self.assertRaisesRegex(FixtureError, "parametrized fixtures"):
            registry(value=pytest.fixture(params=[1])(lambda: 1))

    async def test_close_is_idempotent_and_context_cannot_reopen(self):
        context = FixtureContext(registry())
        await context.close()
        await context.close()
        with self.assertRaisesRegex(FixtureError, "already closed"):
            await context.resolve("tmp_path")

    def test_unsupported_declarations_fail_loudly(self):
        with self.assertRaisesRegex(ValueError, "only scope='function'"):
            fixture(scope="session")
        with self.assertRaisesRegex(ValueError, "parametrized fixtures"):
            fixture(params=[1])


if __name__ == "__main__":
    unittest.main()
