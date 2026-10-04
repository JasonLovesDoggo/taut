"""Contract tests for fixture lifetimes; run with python tests/test_fixture_runtime.py."""

import asyncio
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import threading
import types
import unittest
from unittest import mock
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

    async def test_tmp_path_restores_cwd_from_root_and_descendant(self):
        original_cwd = Path.cwd()
        for relative in (Path("."), Path("child/grandchild")):
            with self.subTest(relative=relative):
                try:
                    async with FixtureContext(registry()) as context:
                        path = await context.resolve("tmp_path")
                        destination = path / relative
                        destination.mkdir(parents=True, exist_ok=True)
                        os.chdir(destination)
                    self.assertEqual(Path.cwd(), original_cwd)
                    self.assertFalse(path.exists())
                finally:
                    os.chdir(original_cwd)

    async def test_tmp_path_preserves_unrelated_cwd_with_same_prefix(self):
        original_cwd = Path.cwd()
        sibling = None
        try:
            async with FixtureContext(registry()) as context:
                path = await context.resolve("tmp_path")
                sibling = path.with_name(path.name + "-other")
                sibling.mkdir()
                os.chdir(sibling)
            self.assertEqual(Path.cwd().resolve(), sibling.resolve())
            self.assertFalse(path.exists())
        finally:
            os.chdir(original_cwd)
            if sibling is not None:
                sibling.rmdir()

    async def test_tmp_path_restores_cwd_through_symlinked_temp_root(self):
        original_cwd = Path.cwd()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target"
            target.mkdir()
            link = root / "link"
            try:
                link.symlink_to(target, target_is_directory=True)
            except OSError as error:
                self.skipTest(f"directory symlinks unavailable: {error}")
            try:
                with mock.patch.object(tempfile, "tempdir", str(link)):
                    async with FixtureContext(registry()) as context:
                        path = await context.resolve("tmp_path")
                        self.assertEqual(path.parent, link)
                        os.chdir(path)
                    self.assertEqual(Path.cwd(), original_cwd)
                    self.assertFalse(path.exists())
            finally:
                os.chdir(original_cwd)

    @unittest.skipIf(os.name == "nt", "Windows cannot remove the current directory")
    async def test_tmp_path_restores_deleted_cwd(self):
        original_cwd = Path.cwd()
        try:
            async with FixtureContext(registry()) as context:
                path = await context.resolve("tmp_path")
                destination = path / "removed"
                destination.mkdir()
                os.chdir(destination)
                destination.rmdir()
            self.assertEqual(Path.cwd(), original_cwd)
            self.assertFalse(path.exists())
        finally:
            os.chdir(original_cwd)

    async def test_tmp_path_reports_cleanup_failure(self):
        temporary = tempfile.TemporaryDirectory()
        try:
            with (
                mock.patch.object(tempfile, "TemporaryDirectory", return_value=temporary),
                mock.patch.object(temporary, "cleanup", side_effect=PermissionError("cleanup denied")),
            ):
                with self.assertRaisesRegex(PermissionError, "cleanup denied"):
                    async with FixtureContext(registry()) as context:
                        await context.resolve("tmp_path")
        finally:
            temporary.cleanup()

    async def test_monkeypatch_undo_after_failure(self):
        key = "TAUT_FIXTURE_TEST_SENTINEL"
        old_value = os.environ.get(key)
        old_cwd = os.getcwd()
        mapping = {"key": 1}
        target = types.SimpleNamespace(value=1)
        cleanup = tempfile.TemporaryDirectory.cleanup

        def cleanup_outside_cwd(directory):
            # Model Windows' cwd lock on every platform, before deletion occurs.
            self.assertFalse(Path.cwd().resolve().is_relative_to(Path(directory.name).resolve()))
            cleanup(directory)

        with (
            mock.patch.object(tempfile.TemporaryDirectory, "cleanup", cleanup_outside_cwd),
            self.assertRaisesRegex(AssertionError, "test failed"),
        ):
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
        self.assertFalse(path.exists())

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

    async def test_package_conftest_supports_parent_relative_and_sibling_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            name = "fixture_package_" + root.name
            package = root / name
            nested = package / "nested"
            nested.mkdir(parents=True)
            (package / "__init__.py").write_text("")
            (nested / "__init__.py").write_text("")
            (package / "shared.py").write_text("PARENT = 10\n")
            (nested / "helper.py").write_text("LOCAL = 20\n")
            sibling_name = "fixture_sibling_" + root.name
            (nested / f"{sibling_name}.py").write_text("SIBLING = 12\n")
            (nested / "conftest.py").write_text(
                "from _fixture_decorators import fixture\n"
                "from ..shared import PARENT\nfrom .helper import LOCAL\n"
                f"from {sibling_name} import SIBLING\n"
                "@fixture\ndef number(): return PARENT + LOCAL + SIBLING\n"
            )
            original_path = sys.path[:]
            module = types.ModuleType("fixture_package_test")
            definitions = FixtureRegistry.from_module(module, nested / "test_example.py", root)
            self.assertEqual(sys.path, original_path)
            async with FixtureContext(definitions) as context:
                self.assertEqual(await context.resolve("number"), 42)
            self.assertEqual(sys.modules[f"{name}.nested.conftest"].__package__, f"{name}.nested")

    async def test_default_root_follows_project_configuration_not_cwd(self):
        with tempfile.TemporaryDirectory() as directory:
            outside = Path(directory).resolve()
            root = outside / "project"
            nested = root / "tests"
            nested.mkdir(parents=True)
            (root / "pyproject.toml").write_text("")
            (outside / "conftest.py").write_text(
                "from _fixture_decorators import fixture\n@fixture\ndef leaked(): return True\n"
            )
            (root / "conftest.py").write_text(
                "from _fixture_decorators import fixture\n@fixture\ndef number(): return 42\n"
            )
            original_cwd = os.getcwd()
            try:
                os.chdir(outside)
                module = types.ModuleType("fixture_root_test")
                definitions = FixtureRegistry.from_module(module, nested / "test_example.py")
            finally:
                os.chdir(original_cwd)
            async with FixtureContext(definitions) as context:
                self.assertEqual(await context.resolve("number"), 42)
                with self.assertRaisesRegex(FixtureError, "not found"):
                    await context.resolve("leaked")
            with self.assertRaisesRegex(FixtureError, "not an ancestor"):
                FixtureRegistry.from_module(module, nested / "test_example.py", root / "elsewhere")

    async def test_venv_only_project_excludes_outer_conftest(self):
        with tempfile.TemporaryDirectory() as directory:
            outer = Path(directory).resolve()
            root = outer / "inner"
            nested = root / "tests"
            nested.mkdir(parents=True)
            (root / ".venv").mkdir()
            (outer / "pyproject.toml").write_text("")
            (outer / "conftest.py").write_text("raise RuntimeError('outer conftest imported')\n")
            (root / "conftest.py").write_text(
                "from _fixture_decorators import fixture\n@fixture\ndef number(): return 42\n"
            )
            definitions = FixtureRegistry.from_module(
                types.ModuleType("venv_root_test"), nested / "test_example.py"
            )
            async with FixtureContext(definitions) as context:
                self.assertEqual(await context.resolve("number"), 42)

    def test_concurrent_conftest_loading_never_exposes_partial_module(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "conftest.py"
            path.write_text("import time\ntime.sleep(0.04)\nready = object()\n")
            barrier = threading.Barrier(8)

            def load_concurrently():
                barrier.wait()
                return runtime._load_conftest(path).ready

            before = sys.path[:]
            with ThreadPoolExecutor(max_workers=8) as pool:
                results = list(pool.map(lambda _: load_concurrently(), range(8)))
            self.assertTrue(all(value is results[0] for value in results))
            self.assertEqual(sys.path, before)

    def test_failed_conftest_can_retry_and_path_cleanup_preserves_original_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "conftest.py"
            path.write_text(
                "import sys\nfrom pathlib import Path\n"
                "sys.path.remove(str(Path(__file__).parent))\n"
                "raise ValueError('original import error')\n"
            )
            before = sys.path[:]
            with self.assertRaisesRegex(ValueError, "original import error"):
                runtime._load_conftest(path)
            self.assertEqual(sys.path, before)
            path.write_text("ready = True\n")
            self.assertTrue(runtime._load_conftest(path).ready)
            self.assertEqual(sys.path, before)

    def test_monkeypatch_removes_inherited_instance_shadows(self):
        class Parent:
            value = 1

            def method(self):
                return self.value

        target = Parent()
        patch = runtime.MonkeyPatch()
        patch.setattr(target, "value", 2)
        patch.setattr(target, "method", lambda: 3)
        self.assertEqual(target.value, 2)
        self.assertEqual(target.method(), 3)
        patch.undo()
        self.assertEqual(vars(target), {})
        Parent.value = 4
        self.assertEqual(target.value, 4)
        self.assertEqual(target.method(), 4)

    def test_monkeypatch_restores_descriptor_values_and_class_descriptors(self):
        class Slotted:
            __slots__ = ("value", "__dict__")

        class Property:
            def __init__(self):
                self._value = 1

            @property
            def value(self):
                return self._value

            @value.setter
            def value(self, value):
                self._value = value

        class Class:
            @classmethod
            def method(cls):
                return 1

        original = Class.__dict__["method"]
        slotted, prop = Slotted(), Property()
        slotted.value = 1
        patch = runtime.MonkeyPatch()
        patch.setattr(slotted, "value", 2)
        patch.setattr(prop, "value", 2)
        patch.setattr(Class, "method", lambda: 2)
        patch.undo()
        self.assertEqual(slotted.value, 1)
        self.assertEqual(prop.value, 1)
        self.assertIs(Class.__dict__["method"], original)

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
