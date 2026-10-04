"""Function-scoped fixtures. Registries are reusable; contexts belong to one test.

Keep this module dependency-free: the worker ships it alongside its embedded
Python source, without importing pytest or the installed taut extension.
"""

import importlib
import importlib.util
import inspect
import os
from pathlib import Path
import sys
import tempfile
import threading


class FixtureError(RuntimeError):
    """A fixture declaration, dependency, or lifecycle error."""


_MISSING = object()
_IMPORT_LOCK = threading.RLock()


class MonkeyPatch:
    """Reversible process-state edits; only safe in a serial worker slot."""

    def __init__(self):
        self._undo = []

    def setattr(self, target, name, value, raising=True):
        old = getattr(target, name, _MISSING)
        if old is _MISSING and raising:
            raise AttributeError(name)
        # Restore the actual attribute owner. A patch of an inherited instance
        # attribute must remove its new shadow; slots and properties instead
        # require restoring their observed value through the descriptor.
        if isinstance(target, type):
            old = target.__dict__.get(name, _MISSING)
        else:
            namespace = getattr(target, "__dict__", None)
            if namespace is not None and name not in namespace:
                descriptor = inspect.getattr_static(type(target), name, None)
                if not hasattr(descriptor, "__set__"):
                    old = _MISSING
        setattr(target, name, value)
        self._undo.append(lambda: delattr(target, name) if old is _MISSING else setattr(target, name, old))

    def delattr(self, target, name, raising=True):
        old = getattr(target, name, _MISSING)
        if old is _MISSING:
            if raising:
                raise AttributeError(name)
            return
        if isinstance(target, type):
            old = target.__dict__.get(name, _MISSING)
        delattr(target, name)
        self._undo.append(lambda: setattr(target, name, old))

    def setitem(self, mapping, name, value):
        old = mapping.get(name, _MISSING)
        mapping[name] = value
        self._undo.append(lambda: mapping.pop(name, None) if old is _MISSING else mapping.__setitem__(name, old))

    def delitem(self, mapping, name, raising=True):
        if name not in mapping:
            if raising:
                raise KeyError(name)
            return
        old = mapping[name]
        del mapping[name]
        self._undo.append(lambda: mapping.__setitem__(name, old))

    def setenv(self, name, value, prepend=None):
        value = str(value)
        if prepend is not None and name in os.environ:
            value += prepend + os.environ[name]
        self.setitem(os.environ, name, value)

    def delenv(self, name, raising=True):
        self.delitem(os.environ, name, raising)

    def chdir(self, path):
        old = os.getcwd()
        os.chdir(path)
        self._undo.append(lambda: os.chdir(old))

    def syspath_prepend(self, path):
        old = sys.path[:]
        sys.path.insert(0, str(path))
        importlib.invalidate_caches()
        self._undo.append(lambda: sys.path.__setitem__(slice(None), old))

    def undo(self):
        errors = []
        while self._undo:
            try:
                self._undo.pop()()
            except BaseException as error:
                errors.append(error)
        _raise_errors("monkeypatch cleanup failed", errors)


def _raise_errors(message, errors):
    if len(errors) == 1:
        raise errors[0]
    if errors:
        raise BaseExceptionGroup(message, errors)


def _tmp_path():
    previous_cwd = Path.cwd()
    with tempfile.TemporaryDirectory(prefix="taut-") as directory:
        path = Path(directory)
        resolved_path = path.resolve()
        try:
            yield path
        finally:
            try:
                current_cwd = Path.cwd().resolve()
            except FileNotFoundError:
                # POSIX permits removing the current directory before teardown.
                os.chdir(previous_cwd)
            else:
                if current_cwd.is_relative_to(resolved_path):
                    # Windows cannot delete cwd or its parents. This must happen
                    # before cleanup even if monkeypatch has not unwound yet.
                    os.chdir(previous_cwd)


def _monkeypatch():
    patch = MonkeyPatch()
    try:
        yield patch
    finally:
        patch.undo()


class _Definition:
    __slots__ = ("function", "serial")

    def __init__(self, function, *, serial=False):
        self.function = function
        self.serial = serial


def _declaration(value):
    metadata = getattr(value, "__taut_fixture__", None)
    if metadata is not None:
        return value, metadata
    marker = getattr(value, "_fixture_function_marker", None)
    if marker is None:
        marker = getattr(value, "_pytestfixturefunction", None)
    if marker is None:
        return None
    function = getattr(value, "_fixture_function", None)
    if function is None:
        wrapped = getattr(value, "__pytest_wrapped__", None)
        function = getattr(wrapped, "obj", value)
    return function, {
        "name": getattr(marker, "name", None),
        "scope": getattr(marker, "scope", "function"),
        "params": getattr(marker, "params", None),
        "autouse": getattr(marker, "autouse", False),
    }


class _ImportPath(str):
    """Identity distinguishes our temporary entry from user edits to sys.path."""


def _load_conftest(path):
    # Module publication and temporary sys.path changes must be atomic with
    # respect to other registry builders, including when imports release the GIL.
    with _IMPORT_LOCK:
        package_parts = []
        import_root = path.parent
        while (import_root / "__init__.py").is_file():
            package_parts.append(import_root.name)
            import_root = import_root.parent
        package = ".".join(reversed(package_parts))
        module_name = (
            package + ".conftest" if package
            else "_taut_conftest_" + str(hash(str(path))).replace("-", "n")
        )
        cached = sys.modules.get(module_name)
        if cached is not None:
            cached_file = getattr(cached, "__file__", None)
            if cached_file is None or Path(cached_file).resolve() != path.resolve():
                raise FixtureError(f"conftest import {module_name!r} conflicts with {cached_file}")
            return importlib.import_module(module_name) if package else cached

        added = []
        for directory in (import_root, path.parent):
            entry = _ImportPath(str(directory))
            if entry not in sys.path:
                sys.path.insert(0, entry)
                added.append(entry)
        try:
            if package:
                # Import package __init__ files normally so relative imports
                # have the same identity and behavior as the test's imports.
                parent_module = importlib.import_module(package)
                expected = (path.parent / "__init__.py").resolve()
                actual = getattr(parent_module, "__file__", None)
                if actual is None or Path(actual).resolve() != expected:
                    raise FixtureError(f"conftest package {package!r} conflicts with {actual}")
                return importlib.import_module(module_name)
            spec = importlib.util.spec_from_file_location(module_name, path)
            module = importlib.util.module_from_spec(spec)
            sys.modules[module_name] = module
            try:
                spec.loader.exec_module(module)
            except BaseException:
                sys.modules.pop(module_name, None)
                raise
            return module
        finally:
            for entry in added:
                for index, current in enumerate(sys.path):
                    if current is entry:
                        del sys.path[index]
                        break


def _project_root(directory):
    """Infer the test project's boundary without consulting mutable cwd."""
    markers = ("pyproject.toml", "pytest.ini", "setup.cfg", "setup.py", ".git")
    for parent in (directory, *directory.parents):
        if any((parent / marker).exists() for marker in markers):
            return parent
    return directory


class FixtureRegistry:
    """Fixture definitions and cached signatures shared across test contexts."""

    def __init__(self):
        self.definitions = {
            "tmp_path": _Definition(_tmp_path),
            "monkeypatch": _Definition(_monkeypatch, serial=True),
        }
        self.autouse = []
        self._signatures = {}

    @classmethod
    def from_module(cls, module, test_file=None, root=None):
        """Collect ancestors within root, or the nearest project configuration."""
        registry = cls()
        test_file = test_file or getattr(module, "__file__", None)
        if test_file:
            directory = Path(test_file).resolve().parent
            boundary = Path(root).resolve() if root is not None else _project_root(directory)
            if boundary != directory and boundary not in directory.parents:
                raise FixtureError(f"fixture root {boundary} is not an ancestor of {directory}")
            ancestors = []
            for parent in (directory, *directory.parents):
                ancestors.append(parent)
                if parent == boundary:
                    break
            for parent in reversed(ancestors):
                path = parent / "conftest.py"
                if path.is_file():
                    registry.add_module(_load_conftest(path))
        registry.add_module(module)
        return registry

    def add_module(self, module):
        for symbol, value in vars(module).items():
            declaration = _declaration(value)
            if declaration is None:
                continue
            function, metadata = declaration
            name = metadata.get("name") or symbol
            if metadata.get("scope", "function") != "function":
                raise FixtureError(f"fixture {name!r}: taut supports only function scope")
            if metadata.get("params") is not None:
                raise FixtureError(f"fixture {name!r}: parametrized fixtures are not yet supported")
            self.definitions[name] = _Definition(function)
            if metadata.get("autouse") and name not in self.autouse:
                self.autouse.append(name)

    def dependencies(self, function):
        # Bound test methods are recreated for each test instance. Cache by the
        # underlying function to avoid retaining every finished instance.
        bound = inspect.ismethod(function)
        key = (function.__func__, True) if bound else (function, False)
        try:
            return self._signatures[key]
        except KeyError:
            names = []
            for parameter in inspect.signature(function).parameters.values():
                if parameter.kind in (parameter.VAR_POSITIONAL, parameter.VAR_KEYWORD):
                    continue
                if parameter.default is not parameter.empty:
                    continue
                if parameter.kind is parameter.POSITIONAL_ONLY:
                    raise FixtureError(f"{function.__name__}: fixture arguments must accept keywords ({parameter.name!r})")
                names.append(parameter.name)
            self._signatures[key] = tuple(names)
            return self._signatures[key]

    def needs_context(self, function, provided=()):
        return bool(self.autouse or any(name not in provided for name in self.dependencies(function)))

    def requires_serial(self, function, provided=()):
        """Whether this test's known dependency graph mutates global state."""
        visited = set()

        def visit(name):
            if name in provided or name in visited:
                return False
            visited.add(name)
            definition = self.definitions.get(name)
            if definition is None:
                return False  # resolve() supplies the actionable missing-name error.
            return definition.serial or any(visit(dep) for dep in self.dependencies(definition.function))

        return any(visit(name) for name in (*self.autouse, *self.dependencies(function)))


class FixtureContext:
    """Resolve and unwind one test's fixtures on the worker's event loop.

    Call ``close`` in a finally block, or use the async context manager. The
    context does not invoke the test; a worker can run synchronous test bodies
    outside an active loop while keeping async setup and cleanup on one loop.
    """

    def __init__(self, registry, *, concurrent=False, provided=None, run_async=None):
        self.registry = registry
        self.concurrent = concurrent
        self.provided = dict(provided or {})
        self._values = dict(self.provided)
        self._run_async = run_async
        self._resolving = []
        self._finalizers = []
        self._closed = False

    async def __aenter__(self):
        return self

    async def __aexit__(self, exc_type, exception, traceback):
        try:
            await self.close()
        except BaseException as teardown:
            if exception is not None:
                raise BaseExceptionGroup("test and fixture teardown failed", [exception, teardown]) from None
            raise
        return False

    async def kwargs(self, function):
        if self.concurrent and self.registry.requires_serial(function, self.provided):
            raise FixtureError("monkeypatch changes process-global state; run this test in a serial worker slot")
        for name in self.registry.autouse:
            await self.resolve(name)
        result = {name: await self.resolve(name) for name in self.registry.dependencies(function)}
        result.update(self.provided)
        return result

    def _await(self, value):
        if self._run_async is None:
            if inspect.iscoroutine(value):
                value.close()
            raise FixtureError("async fixture requires an event-loop adapter")
        return self._run_async(value)

    def kwargs_sync(self, function):
        if self.concurrent and self.registry.requires_serial(function, self.provided):
            raise FixtureError("monkeypatch changes process-global state; run this test in a serial worker slot")
        for name in self.registry.autouse:
            self.resolve_sync(name)
        result = {name: self.resolve_sync(name) for name in self.registry.dependencies(function)}
        result.update(self.provided)
        return result

    def resolve_sync(self, name):
        if self._closed:
            raise FixtureError("fixture context is already closed")
        if name in self._values:
            return self._values[name]
        if name in self._resolving:
            raise FixtureError("fixture dependency cycle: " + " -> ".join((*self._resolving, name)))
        definition = self.registry.definitions.get(name)
        if definition is None:
            available = ", ".join(sorted(self.registry.definitions))
            raise FixtureError(f"fixture {name!r} not found; available: {available}")
        if self.concurrent and definition.serial:
            raise FixtureError(f"fixture {name!r} changes process-global state; run this test in a serial worker slot")
        self._resolving.append(name)
        try:
            kwargs = {dep: self.resolve_sync(dep) for dep in self.registry.dependencies(definition.function)}
            value = definition.function(**kwargs)
            if inspect.isawaitable(value):
                value = self._await(value)
            if inspect.isasyncgen(value):
                generator = value
                try:
                    value = self._await(anext(generator))
                except StopAsyncIteration:
                    raise FixtureError(f"fixture {name!r} did not yield a value") from None
                self._finalizers.append((name, generator, True))
            elif inspect.isgenerator(value):
                generator = value
                try:
                    value = next(generator)
                except StopIteration:
                    raise FixtureError(f"fixture {name!r} did not yield a value") from None
                self._finalizers.append((name, generator, False))
            self._values[name] = value
            return value
        finally:
            self._resolving.pop()

    def close_sync(self):
        if self._closed:
            return
        self._closed = True
        errors = []
        while self._finalizers:
            name, generator, asynchronous = self._finalizers.pop()
            try:
                try:
                    self._await(anext(generator)) if asynchronous else next(generator)
                except (StopIteration, StopAsyncIteration):
                    pass
                else:
                    raise FixtureError(f"fixture {name!r} yielded more than once")
            except BaseException as error:
                errors.append(error)
            finally:
                try:
                    self._await(generator.aclose()) if asynchronous else generator.close()
                except BaseException as error:
                    errors.append(error)
        self._values.clear()
        _raise_errors("fixture teardown failed", errors)

    async def resolve(self, name):
        if self._closed:
            raise FixtureError("fixture context is already closed")
        if name in self._values:
            return self._values[name]
        if name in self._resolving:
            chain = " -> ".join((*self._resolving, name))
            raise FixtureError(f"fixture dependency cycle: {chain}")
        definition = self.registry.definitions.get(name)
        if definition is None:
            chain = " -> ".join((*self._resolving, name))
            available = ", ".join(sorted(self.registry.definitions))
            raise FixtureError(f"fixture {name!r} not found (dependency: {chain}); available: {available}")
        if self.concurrent and definition.serial:
            raise FixtureError(f"fixture {name!r} changes process-global state; run this test in a serial worker slot")
        self._resolving.append(name)
        try:
            kwargs = {dep: await self.resolve(dep) for dep in self.registry.dependencies(definition.function)}
            value = definition.function(**kwargs)
            if inspect.isawaitable(value):
                value = await value
            if inspect.isasyncgen(value):
                generator = value
                try:
                    value = await anext(generator)
                except StopAsyncIteration:
                    raise FixtureError(f"fixture {name!r} did not yield a value") from None
                self._finalizers.append((name, generator, True))
            elif inspect.isgenerator(value):
                generator = value
                try:
                    value = next(generator)
                except StopIteration:
                    raise FixtureError(f"fixture {name!r} did not yield a value") from None
                self._finalizers.append((name, generator, False))
            self._values[name] = value
            return value
        finally:
            self._resolving.pop()

    async def close(self):
        if self._closed:
            return
        self._closed = True
        errors = []
        while self._finalizers:
            name, generator, asynchronous = self._finalizers.pop()
            try:
                if asynchronous:
                    try:
                        await anext(generator)
                    except StopAsyncIteration:
                        pass
                    else:
                        raise FixtureError(f"fixture {name!r} yielded more than once")
                else:
                    try:
                        next(generator)
                    except StopIteration:
                        pass
                    else:
                        raise FixtureError(f"fixture {name!r} yielded more than once")
            except BaseException as error:
                errors.append(error)
            finally:
                try:
                    if asynchronous:
                        await generator.aclose()
                    else:
                        generator.close()
                except BaseException as error:
                    errors.append(error)
        self._values.clear()
        _raise_errors("fixture teardown failed", errors)
