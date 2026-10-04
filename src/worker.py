"""Dependency-free Python execution worker. Test code never writes to the protocol fd."""
import contextvars
import importlib.util
import importlib.machinery
import io
import json
import os
import struct
import sys
import tempfile
import threading
import weakref
import time
import traceback
import types

_MAX_OUTPUT = 1024 * 1024
_protocol = os.fdopen(os.dup(1), "wb")
_input = os.fdopen(os.dup(0), "rb")
with open(os.devnull, "rb") as _null:
    os.dup2(_null.fileno(), 0)
sys.stdin = io.TextIOWrapper(os.fdopen(os.dup(0), "rb"))
# dup() descriptors are non-inheritable: child processes cannot keep our protocol open.
_native = [tempfile.TemporaryFile() if hasattr(os, "pread") else tempfile.NamedTemporaryFile() for _ in range(2)]
# Windows has no pread. Reopen named captures with independent read offsets and
# O_TEMPORARY sharing so native writes never race with a seek on the writer fd.
_native_readers = None if hasattr(os, "pread") else [
    os.open(file.name, os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_TEMPORARY", 0))
    for file in _native
]
_native_offsets = [0, 0]
for _fd, _file in enumerate(_native, 1):
    os.dup2(_file.fileno(), _fd)
_capture = contextvars.ContextVar("taut_capture", default=None)
_owner = contextvars.ContextVar("taut_owner", default=None)
_modules = {}
_loop = None
_asyncio = None
_single_owner = None
_running_cases = set()
_dirty = False
_fixture_runtime = None
_registries = {}
_fixture_directories = {}
_signature_names = {}
_INITIAL_CWD = os.getcwd()
_UNSET = object()


_original_get_code = importlib.machinery.SourceFileLoader.get_code
_library_roots = tuple(os.path.normcase(os.path.abspath(path)) + os.sep for path in (
    os.path.dirname(os.__file__), *(entry for entry in sys.path if entry.endswith(("site-packages", "dist-packages")))
))


def _current_source_code(loader, fullname):
    filename = loader.path
    normalized = os.path.normcase(os.path.abspath(filename))
    if not normalized.startswith(_library_roots):
        return loader.source_to_code(loader.get_data(filename), filename)
    return _original_get_code(loader, fullname)


# Project source can change twice within a filesystem timestamp second. Loading
# its existing timestamp pyc would silently miss edits during watch or reruns.
importlib.machinery.SourceFileLoader.get_code = _current_source_code


class Capture(io.StringIO):
    def __init__(self):
        super().__init__()
        self.size = 0
        self.truncated = False

    def write(self, value):
        if not isinstance(value, str):
            raise TypeError("write() argument must be str")
        room = _MAX_OUTPUT - self.size
        if room > 0:
            part = value[:room]
            super().write(part)
            self.size += len(part)
        if len(value) > room and not self.truncated:
            super().write("\n[taut: output truncated after 1 MiB]\n")
            self.truncated = True
        return len(value)


_shared = [Capture(), Capture()]


class BinaryOutput:
    def __init__(self, text):
        self.text = text

    def write(self, data):
        self.text.write(bytes(data).decode("utf-8", "replace"))
        return len(data)

    def flush(self):
        pass

    def fileno(self):
        return self.text.fileno()


class Output:
    encoding = "utf-8"
    errors = "replace"
    closed = False

    def __init__(self, index):
        self.index = index
        self.buffer = BinaryOutput(self)

    def write(self, value):
        buffers = _capture.get()
        if buffers is None and _single_owner is not None:
            buffers = _single_owner.buffers
        return (buffers or _shared)[self.index].write(value)

    def writelines(self, values):
        for value in values:
            self.write(value)

    def flush(self):
        pass

    def isatty(self):
        return False

    def fileno(self):
        return self.index + 1

    def writable(self):
        return True


_outputs = [Output(0), Output(1)]
sys.stdout, sys.stderr = _outputs
sys.__stdout__, sys.__stderr__ = _outputs


_original_thread_start = threading.Thread.start
_original_thread_error = threading.excepthook


def _thread_start(thread, *args, **kwargs):
    owner = _owner.get() or _single_owner
    target = getattr(thread, "_target", None)
    if owner is not None and getattr(target, "__module__", "") != "concurrent.futures.thread":
        thread._taut_owner = weakref.ref(owner)
    return _original_thread_start(thread, *args, **kwargs)


def _thread_error(args):
    reference = getattr(args.thread, "_taut_owner", None)
    owner = reference() if reference is not None else None
    if owner is not None:
        owner.record(args.exc_value, "thread")
    _original_thread_error(args)


threading.Thread.start = _thread_start
threading.excepthook = _thread_error


def _send(value):
    data = json.dumps(value, ensure_ascii=True, separators=(",", ":")).encode()
    _protocol.write(struct.pack("<I", len(data)))
    _protocol.write(data)
    _protocol.flush()


def _read():
    header = _input.read(4)
    if not header:
        return None
    if len(header) != 4:
        raise EOFError("Incomplete request header")
    length = struct.unpack("<I", header)[0]
    if length > 64 * 1024 * 1024:
        raise ValueError("Request exceeds 64 MiB")
    data = _input.read(length)
    if len(data) != length:
        raise EOFError("Incomplete request")
    return json.loads(data)


def _load_module(filename):
    module = _modules.get(filename)
    if module is not None:
        return module
    directory = os.path.dirname(filename)
    parts = [os.path.splitext(os.path.basename(filename))[0]]
    parent = directory
    while os.path.isfile(os.path.join(parent, "__init__.py")):
        parts.insert(0, os.path.basename(parent))
        parent = os.path.dirname(parent)
    for path in (directory, parent):
        if path not in sys.path:
            sys.path.insert(0, path)
    if len(parts) > 1:
        name = ".".join(parts)
        __import__(".".join(parts[:-1]))
    else:
        name = "_taut_test_" + str(abs(hash(filename)))
    existing = sys.modules.get(name)
    if existing is not None:
        if os.path.abspath(getattr(existing, "__file__", "")) != filename:
            raise ImportError("Test module name collision for " + name)
        module = existing
    else:
        spec = importlib.util.spec_from_file_location(name, filename)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        try:
            spec.loader.exec_module(module)
        except BaseException:
            sys.modules.pop(name, None)
            raise
    _modules[filename] = module
    return module


def _required_names(function):
    underlying = getattr(function, "__func__", function)
    seen = set()
    while hasattr(underlying, "__wrapped__") and id(underlying) not in seen:
        seen.add(id(underlying))
        underlying = underlying.__wrapped__
    key = (underlying, getattr(function, "__self__", None) is not None)
    if key not in _signature_names:
        code = getattr(underlying, "__code__", None)
        if code is None:
            return None
        start = int(key[1])
        stop = code.co_argcount - len(getattr(underlying, "__defaults__", None) or ())
        names = list(code.co_varnames[start:stop])
        defaults = getattr(underlying, "__kwdefaults__", None) or {}
        names.extend(name for name in code.co_varnames[code.co_argcount:code.co_argcount + code.co_kwonlyargcount] if name not in defaults)
        _signature_names[key] = tuple(names)
    return _signature_names[key]


def _fixture_boundary(filename):
    directory = os.path.dirname(filename)
    if directory not in _fixture_directories:
        parents = []
        parent = directory
        while True:
            parents.append(parent)
            if any(os.path.exists(os.path.join(parent, marker)) for marker in ("pyproject.toml", "pytest.ini", "setup.cfg", "setup.py", ".git")):
                break
            next_parent = os.path.dirname(parent)
            if next_parent == parent:
                parents = parents[:parents.index(_INITIAL_CWD) + 1] if _INITIAL_CWD in parents else [directory]
                break
            parent = next_parent
        _fixture_directories[directory] = (parents[-1], any(os.path.isfile(os.path.join(parent, "conftest.py")) for parent in parents))
    return _fixture_directories[directory]


def _fixtures():
    global _fixture_runtime
    if _fixture_runtime is None:
        module = types.ModuleType("_taut_fixtures")
        exec(compile(bytes.fromhex(_FIXTURES_SOURCE_HEX).decode("utf-8"), "<taut fixtures>", "exec"), module.__dict__)
        _fixture_runtime = module
    return _fixture_runtime


def _registry_for(module, filename, function, provided):
    registry = _registries.get(filename, _UNSET)
    names = _required_names(function)
    needs_arguments = names is None or any(name not in provided for name in names)
    if registry is _UNSET:
        root, conftest = _fixture_boundary(filename)
        local = any(getattr(value, "__taut_fixture__", None) is not None or getattr(value, "_fixture_function_marker", None) is not None or getattr(value, "_pytestfixturefunction", None) is not None for value in vars(module).values())
        registry = _fixtures().FixtureRegistry.from_module(module, filename, root) if conftest or local or needs_arguments else None
        _registries[filename] = registry
    elif registry is None and needs_arguments:
        root, _ = _fixture_boundary(filename)
        registry = _fixtures().FixtureRegistry.from_module(module, filename, root)
        _registries[filename] = registry
    return registry


def _hook_call(function, target):
    import inspect
    signature = inspect.signature(function)
    try:
        signature.bind(target)
    except TypeError:
        signature.bind()
        return function()
    return function(target)


def _pytest_marks(value):
    marks = getattr(value, "pytestmark", ())
    return marks if isinstance(marks, (tuple, list)) else (marks,)


def _runtime_skip(value, module):
    for mark in _pytest_marks(value):
        name = getattr(mark, "name", None)
        if name not in ("skip", "skipif"):
            continue
        kwargs = getattr(mark, "kwargs", {})
        args = getattr(mark, "args", ())
        if name == "skip":
            return kwargs.get("reason") or (str(args[0]) if args else "pytest skip")
        conditions = args or (kwargs.get("condition", False),)
        for condition in conditions:
            if isinstance(condition, str):
                condition = eval(condition, vars(module))
            if condition:
                return kwargs.get("reason", "pytest skipif")
    return None


def _focus_traceback(summary, exc):
    # Filter structured frames, never rendered text: exception messages, notes,
    # and even user-compiled filenames may contain identical-looking text.
    pending = [(summary, exc)]
    seen = set()
    while pending:
        current, error = pending.pop()
        if id(current) in seen:
            continue
        seen.add(id(current))
        frames = []
        for frame, (live_frame, _) in zip(current.stack, traceback.walk_tb(error.__traceback__)):
            internal = (
                live_frame.f_globals is globals() and frame.filename == "<taut worker>"
            ) or (
                _fixture_runtime is not None
                and live_frame.f_globals is vars(_fixture_runtime)
                and frame.filename == "<taut fixtures>"
            )
            if not internal:
                frames.append(frame)
        current.stack = traceback.StackSummary.from_list(frames)
        for attribute in ("__cause__", "__context__"):
            child = getattr(current, attribute, None)
            if child is not None:
                pending.append((child, getattr(error, attribute)))
        if current.exceptions is not None:
            pending.extend(zip(current.exceptions, error.exceptions))


def _error(exc, stage):
    message = str(exc) or ("Assertion failed" if isinstance(exc, AssertionError) else type(exc).__name__)
    summary = traceback.TracebackException.from_exception(exc, capture_locals=False, compact=True)
    raw = "".join(summary.format())
    _focus_traceback(summary, exc)
    return {"message": stage + ": " + type(exc).__name__ + ": " + message,
            "traceback": raw, "focused_traceback": "".join(summary.format())}


def _is_skip(exc):
    return any((base.__name__ == "SkipTest" and base.__module__ == "unittest.case") or (base.__name__ == "Skipped" and base.__module__ == "_pytest.outcomes")
               for base in type(exc).__mro__)


def _check_return(value):
    if isinstance(value, (types.GeneratorType, types.AsyncGeneratorType)):
        if isinstance(value, types.GeneratorType):
            value.close()
        raise TypeError("Generator tests are unsupported; use a normal or async test function")
    return value


def _completed_return(value):
    value = _check_return(value)
    if hasattr(value, "__await__"):
        if isinstance(value, types.CoroutineType):
            value.close()
        raise TypeError("Test returned an unawaited coroutine; await it inside the test")
    return value


def _is_async(function):
    return bool(getattr(getattr(function, "__code__", None), "co_flags", 0) & 0x80)


class Case:
    def __init__(self, request, coverage):
        self.request = request
        self.start = time.perf_counter()
        self.buffers = [Capture(), Capture()]
        self.errors = []
        self.skip = None
        self.target = None
        self.instance = None
        self.unittest = False
        self.isolated_asyncio = False
        self.context = None
        self.kwargs = dict(request.get("parameters") or {})
        self.serial = False
        self.setup_hook = None
        self.teardown_hook = None
        self.tasks = set()
        self.generators = set()
        self.coverage = {} if coverage else None

    def record(self, exc, stage):
        if _is_skip(exc) and not self.errors:
            self.skip = str(exc)
        else:
            self.errors.append(_error(exc, stage))

    def prepare(self):
        token = _capture.set(self.buffers)
        owner = _owner.set(self)
        try:
            module = _load_module(self.request["file"])
            cls = getattr(module, self.request["class"]) if self.request.get("class") else None
            target = getattr(cls or module, self.request["function"])
            declarations = []
            for value in (cls, target):
                declarations.extend(metadata["argnames"] for metadata in getattr(value, "_taut_parametrize", ()))
                for mark in _pytest_marks(value):
                    if getattr(mark, "name", None) == "parametrize":
                        args = getattr(mark, "args", ())
                        declarations.append(args[0] if args else mark.kwargs["argnames"])
            names = []
            for declaration in declarations:
                names.extend(part.strip() for part in declaration.split(",")) if isinstance(declaration, str) else names.extend(declaration)
            if declarations and (len(set(names)) != len(names) or set(names) != set(self.kwargs)):
                raise TypeError("Parametrize decorator was not fully expanded during collection; use @parametrize or @taut.parametrize directly with literal values and unique argument names")
            for value in (module, cls, target):
                for mark in _pytest_marks(value):
                    if getattr(mark, "name", None) in ("usefixtures", "xfail"):
                        raise TypeError("pytest " + mark.name + " metadata is unsupported; use explicit fixture arguments or supported unittest expected-failure semantics")
                if getattr(value, "__unittest_skip__", False) or getattr(value, "_taut_skip", False):
                    self.skip = getattr(value, "__unittest_skip_why__", None) or getattr(value, "_taut_skip_reason", "")
                    return
                reason = _runtime_skip(value, module)
                if reason is not None:
                    self.skip = reason
                    return
            if cls is not None:
                if any(base.__name__ == "TestCase" and base.__module__ == "unittest.case" for base in cls.__mro__):
                    self.instance = cls(self.request["function"])
                    self.unittest = True
                    self.isolated_asyncio = any(base.__name__ == "IsolatedAsyncioTestCase" and base.__module__ == "unittest.async_case" for base in cls.__mro__)
                else:
                    self.instance = cls()
                target = getattr(self.instance, self.request["function"])
            self.target = target
            for name in ("setup_module", "teardown_module", "setUpModule", "tearDownModule"):
                if callable(getattr(module, name, None)):
                    raise TypeError(name + " hooks are unsupported; use function-scoped @fixture setup and teardown")
            if cls is not None:
                for base in cls.__mro__:
                    for name, value in vars(base).items():
                        function = getattr(value, "__func__", value)
                        if any(getattr(function, attribute, None) is not None for attribute in ("__taut_fixture__", "_fixture_function_marker", "_pytestfixturefunction")):
                            raise TypeError("Class fixture " + name + " is unsupported; declare function-scoped fixtures at module level")
                for name in ("setup_class", "teardown_class", "setUpClass", "tearDownClass"):
                    value = getattr(cls, name, None)
                    if callable(value) and not (self.unittest and getattr(value, "__module__", "") == "unittest.case"):
                        raise TypeError(name + " hooks are unsupported; use function-scoped setup or fixtures")
            if not self.unittest:
                self.setup_hook = getattr(self.instance, "setup_method", None) if self.instance is not None else getattr(module, "setup_function", None)
                self.teardown_hook = getattr(self.instance, "teardown_method", None) if self.instance is not None else getattr(module, "teardown_function", None)
            registry = _registry_for(module, self.request["file"], target, self.kwargs)
            if registry is not None and registry.needs_context(target, self.kwargs):
                self.serial = registry.requires_serial(target, self.kwargs)
                self.context = _fixtures().FixtureContext(registry, provided=self.kwargs, run_async=lambda value: _await_value(value, self))
        except BaseException as exc:
            self.record(exc, "collection")
        finally:
            _owner.reset(owner)
            _capture.reset(token)

    def result(self, shared=False):
        output = [buffer.getvalue() for buffer in self.buffers]
        for index in (0, 1):
            # pread leaves the writer's file offset untouched, including subprocess writes.
            end = os.fstat(_native[index].fileno()).st_size
            size = end - _native_offsets[index]
            if size and _native_readers is None:
                native = os.pread(_native[index].fileno(), min(size, _MAX_OUTPUT), _native_offsets[index])
            elif size:
                os.lseek(_native_readers[index], _native_offsets[index], os.SEEK_SET)
                native = os.read(_native_readers[index], min(size, _MAX_OUTPUT))
            else:
                native = b""
            _native_offsets[index] = end
            extra = _shared[index].getvalue() + native.decode("utf-8", "replace")
            _shared[index].seek(0)
            _shared[index].truncate(0)
            _shared[index].size = 0
            _shared[index].truncated = False
            if size > _MAX_OUTPUT:
                extra += "\n[taut: native output truncated after 1 MiB]\n"
            if extra:
                output[index] += ("\n[taut: shared worker output; native/thread output cannot be attributed during async overlap]\n" if shared else "") + extra
        error = None
        if self.errors:
            error = {"message": "; ".join(e["message"] for e in self.errors),
                     "traceback": "\n".join(e["traceback"] for e in self.errors),
                     "focused_traceback": "\n".join(e["focused_traceback"] for e in self.errors)}
        result = {"id": self.request["id"], "passed": error is None,
                  "error": error, "skipped": self.skip is not None and error is None,
                  "skip_reason": self.skip, "stdout": output[0], "stderr": output[1],
                  "duration_sec": time.perf_counter() - self.start}
        if self.coverage is not None:
            result["coverage"] = {filename: sorted(lines) for filename, lines in self.coverage.items()}
        return result


def _instrument_loop(loop):
    import asyncio
    def owned_context(callback, context):
        owner = _owner.get()
        if owner is None or context is None or context.get(_owner) is not None:
            return context
        function = getattr(callback, "func", callback)
        # asyncio explicitly preserves the awaiting task's context. Do not
        # inject a child's context into its parent's completion callbacks.
        if isinstance(getattr(function, "__self__", None), asyncio.Task) or getattr(function, "__module__", "").startswith("asyncio"):
            return context
        context = context.copy()
        context.run(_owner.set, owner)
        context.run(_capture.set, owner.buffers)
        return context

    original_soon = loop.call_soon
    original_at = loop.call_at
    original_firstiter = loop._asyncgen_firstiter_hook

    def call_soon(callback, *args, context=None):
        return original_soon(callback, *args, context=owned_context(callback, context))

    def call_at(when, callback, *args, context=None):
        return original_at(when, callback, *args, context=owned_context(callback, context))

    def firstiter(generator):
        original_firstiter(generator)
        owner = _owner.get()
        if owner is not None:
            owner.generators.add(generator)

    loop.call_soon = call_soon
    loop.call_at = call_at
    loop._asyncgen_firstiter_hook = firstiter

    def factory(loop, coro, **kwargs):
        context = kwargs.get("context")
        owner = (context.get(_owner) if context is not None else None) or _owner.get()
        if context is not None and owner is not None:
            context = context.copy()
            context.run(_owner.set, owner)
            context.run(_capture.set, owner.buffers)
            kwargs["context"] = context
        task = asyncio.Task(coro, loop=loop, **kwargs)
        if owner is not None:
            owner.tasks.add(task)
        return task

    def handle_error(loop, context):
        error = context.get("exception") or RuntimeError(context.get("message", "Unhandled event loop error"))
        owner = _owner.get()
        targets = [owner] if owner is not None else list(_running_cases)
        if targets:
            for case in targets:
                case.record(error, "event loop callback" if owner is not None else "unattributed event loop callback")
        else:
            loop.default_exception_handler(context)

    loop.set_exception_handler(handle_error)
    loop.set_task_factory(factory)

def _ensure_loop():
    global _loop, _asyncio
    if _loop is None:
        import asyncio
        _asyncio = asyncio
        _loop = asyncio.new_event_loop()
        _instrument_loop(_loop)
    return _loop


async def _call_async(function):
    value = _check_return(function())
    if hasattr(value, "__await__"):
        return _completed_return(await value)
    return value


async def _cleanup_tasks(case):
    global _dirty
    deadline = time.monotonic() + 0.25
    current = _asyncio.current_task()
    # Give call_soon callbacks one turn so assertions cannot disappear after a pass.
    await _asyncio.sleep(0)
    while True:
        tasks = [task for task in case.tasks if task is not current]
        case.tasks.difference_update(tasks)
        generators = [generator for generator in case.generators if generator.ag_frame is not None]
        case.generators.clear()
        if not tasks and not generators:
            break
        # asyncio clears _log_traceback when the test has retrieved an exception.
        # Re-failing intentionally caught task exceptions would be a false failure.
        report = {task for task in tasks if not task.done() or getattr(task, "_log_traceback", False)}
        for task in tasks:
            if not task.done():
                task.cancel()
        closing = {_asyncio.ensure_future(generator.aclose()) for generator in generators}
        case.tasks.difference_update(closing)
        report.update(closing)
        tasks.extend(closing)
        done, pending = await _asyncio.wait(tasks, timeout=max(0, deadline - time.monotonic()))
        for task in done:
            if not task.cancelled():
                value = task.exception()
                if task in report and value is not None:
                    case.record(value, "async generator cleanup" if task in closing else "background task")
        if pending or (case.tasks and time.monotonic() >= deadline):
            for task in pending:
                task.cancel()
            case.record(RuntimeError("Background tasks did not stop within 250ms after cancellation; worker will be replaced"), "cleanup")
            _dirty = True
            break
    # Timer callbacks retain a test's context even after its coroutine returns.
    # Cancel owned timers so they cannot mutate state during a subsequent test.
    loop = _asyncio.get_running_loop()
    for handle in getattr(loop, "_scheduled", ()):
        context = getattr(handle, "_context", None)
        if context is not None and context.get(_owner) is case:
            handle.cancel()


async def _async_lifecycle(case):
    setup_ok = False
    async_setup_ok = False
    instance = case.instance
    hook_ok = False
    try:
        if case.context is not None:
            case.kwargs = await case.context.kwargs(case.target)
        if case.setup_hook is not None:
            await _call_async(lambda: _hook_call(case.setup_hook, case.target))
        hook_ok = True
        if instance is not None and hasattr(instance, "setUp"):
            await _call_async(instance.setUp)
        setup_ok = True
        if instance is not None and hasattr(instance, "asyncSetUp"):
            await _call_async(instance.asyncSetUp)
        async_setup_ok = True
        await _call_async(lambda: case.target(**case.kwargs))
    except _asyncio.CancelledError:
        raise
    except BaseException as exc:
        case.record(exc, "test" if async_setup_ok else "setup")
    finally:
        for name, enabled in (("asyncTearDown", async_setup_ok), ("tearDown", setup_ok and async_setup_ok)):
            if instance is not None and enabled and hasattr(instance, name):
                try:
                    await _call_async(getattr(instance, name))
                except _asyncio.CancelledError:
                    raise
                except BaseException as exc:
                    case.record(exc, name)
        if instance is not None:
            for cleanup, args, kwargs in reversed(getattr(instance, "_cleanups", [])):
                try:
                    await _call_async(lambda: cleanup(*args, **kwargs))
                except BaseException as exc:
                    case.record(exc, "cleanup")
        if hook_ok and case.teardown_hook is not None:
            try:
                await _call_async(lambda: _hook_call(case.teardown_hook, case.target))
            except BaseException as exc:
                case.record(exc, "teardown hook")
        if case.context is not None:
            try:
                await case.context.close()
            except BaseException as exc:
                case.record(exc, "fixture teardown")
        await _cleanup_tasks(case)


async def _async_case(case, timeout, shared):
    case.start = time.perf_counter()
    token = _capture.set(case.buffers)
    owner = _owner.set(case)
    _running_cases.add(case)
    try:
        if timeout is None:
            await _async_lifecycle(case)
        else:
            await _asyncio.wait_for(_async_lifecycle(case), timeout)
    except BaseException as exc:
        case.record(exc, "timeout" if isinstance(exc, TimeoutError) else "test")
    finally:
        _owner.reset(owner)
        _capture.reset(token)
        _send(case.result(shared))
        _running_cases.discard(case)


def _await_value(value, case):
    loop = _ensure_loop()
    async def await_value():
        return _completed_return(await value)
    task = _asyncio.ensure_future(await_value(), loop=loop)
    try:
        return loop.run_until_complete(task)
    finally:
        case.tasks.discard(task)


def _call_sync(function, case):
    value = _check_return(function())
    return _await_value(value, case) if hasattr(value, "__await__") else value


def _run_unittest(case):
    global _asyncio, _dirty
    import functools
    import unittest

    class Result(unittest.TestResult):
        def addError(self, test, error):
            case.record(error[1], "unittest")
            super().addError(test, error)

        def addFailure(self, test, error):
            case.record(error[1], "unittest")
            super().addFailure(test, error)

        def addSubTest(self, test, subtest, error):
            if error is not None:
                case.record(error[1], "subtest " + str(subtest))
            super().addSubTest(test, subtest, error)

        def addSkip(self, test, reason):
            case.skip = reason
            super().addSkip(test, reason)

        def addExpectedFailure(self, test, error):
            case.skip = "expected failure: " + str(error[1])
            super().addExpectedFailure(test, error)

        def addUnexpectedSuccess(self, test):
            case.record(AssertionError("Expected failure unexpectedly passed"), "unittest")
            super().addUnexpectedSuccess(test)

    if case.isolated_asyncio:
        import asyncio
        _asyncio = asyncio

        class OwnedRunner(asyncio.Runner):
            def run(self, coroutine, *, context=None):
                try:
                    return super().run(coroutine, context=context)
                finally:
                    case.tasks.difference_update(task for task in tuple(case.tasks) if task.get_coro() is coroutine)

            def close(self):
                global _dirty
                if getattr(self, "_taut_closed", False):
                    return
                self._taut_closed = True
                loop = self.get_loop()
                try:
                    self.run(_cleanup_tasks(case))
                    if _dirty:
                        loop.close()
                    else:
                        super().close()
                finally:
                    case.tasks.difference_update(task for task in tuple(case.tasks) if task.get_loop() is loop)

        def make_loop():
            loop = asyncio.new_event_loop()
            _instrument_loop(loop)
            return loop

        def setup_runner():
            case.instance._asyncioRunner = OwnedRunner(debug=True, loop_factory=make_loop)

        case.instance._setupAsyncioRunner = setup_runner

    target = case.target
    if case.isolated_asyncio and _is_async(target):
        @functools.wraps(target)
        async def invoke():
            if case.context is not None:
                case.instance.addAsyncCleanup(case.context.close)
                case.kwargs = await case.context.kwargs(target)
            return _completed_return(await target(**case.kwargs))
    else:
        @functools.wraps(target)
        def invoke():
            if _is_async(target):
                raise TypeError("async unittest methods require unittest.IsolatedAsyncioTestCase")
            if case.context is not None:
                case.instance.addCleanup(case.context.close_sync)
                case.kwargs = case.context.kwargs_sync(target)
            return _completed_return(target(**case.kwargs))
    setattr(case.instance, case.request["function"], invoke)
    if not case.isolated_asyncio:
        def checked(function):
            @functools.wraps(function)
            def call(*args, **kwargs):
                return _completed_return(function(*args, **kwargs))
            return call
        case.instance.setUp = checked(case.instance.setUp)
        case.instance.tearDown = checked(case.instance.tearDown)
        case.instance._callCleanup = lambda function, *args, **kwargs: _completed_return(function(*args, **kwargs))
    result = Result()
    _completed_return(case.instance.run(result))
    if result.testsRun != 1:
        raise TypeError("unittest.TestCase.run must report exactly one test to its result")


def _sync_case(case):
    global _single_owner
    case.start = time.perf_counter()
    token = _capture.set(case.buffers)
    owner = _owner.set(case)
    _single_owner = case
    setup_ok = False
    hook_ok = False
    try:
        if case.unittest:
            _run_unittest(case)
        else:
            if case.context is not None:
                case.kwargs = case.context.kwargs_sync(case.target)
            if case.setup_hook is not None:
                _call_sync(lambda: _hook_call(case.setup_hook, case.target), case)
            hook_ok = True
        if not case.unittest and case.instance is not None and hasattr(case.instance, "setUp"):
            _call_sync(case.instance.setUp, case)
        setup_ok = not case.unittest
        if not case.unittest:
            _call_sync(lambda: case.target(**case.kwargs), case)
    except BaseException as exc:
        case.record(exc, "test" if setup_ok else "setup")
    finally:
        if setup_ok and case.instance is not None and hasattr(case.instance, "tearDown"):
            try:
                _call_sync(case.instance.tearDown, case)
            except BaseException as exc:
                case.record(exc, "tearDown")
        if not case.unittest and case.instance is not None:
            for cleanup, args, kwargs in reversed(getattr(case.instance, "_cleanups", [])):
                try:
                    _call_sync(lambda: cleanup(*args, **kwargs), case)
                except BaseException as exc:
                    case.record(exc, "cleanup")
        if hook_ok and case.teardown_hook is not None:
            try:
                _call_sync(lambda: _hook_call(case.teardown_hook, case.target), case)
            except BaseException as exc:
                case.record(exc, "teardown hook")
        if case.context is not None and not case.unittest:
            try:
                case.context.close_sync()
            except BaseException as exc:
                case.record(exc, "fixture teardown")
        if case.tasks or case.generators:
            _ensure_loop().run_until_complete(_cleanup_tasks(case))
        _single_owner = None
        _owner.reset(owner)
        _capture.reset(token)
        _send(case.result())
        sys.stdout, sys.stderr = _outputs


def _trace(frame, event, arg):
    if event == "line":
        owner = _owner.get() or _single_owner
        if owner is not None and owner.coverage is not None:
            filename = frame.f_code.co_filename
            if not filename.startswith("<") and not any(part in filename for part in ("site-packages", "lib/python", "/usr/lib")):
                owner.coverage.setdefault(os.path.abspath(filename), set()).add(frame.f_lineno)
    return _trace


def _run_batch(request):
    coverage = request.get("collect_coverage", False)
    concurrency = max(1, request.get("async_concurrency", 1))
    timeout = request.get("timeout")
    pending = []
    if coverage:
        sys.settrace(_trace)

    def flush():
        if pending:
            loop = _ensure_loop()
            for case in pending:
                if case.context is not None:
                    case.context.concurrent = len(pending) > 1
            async def run():
                await _asyncio.gather(*(_async_case(case, timeout, len(pending) > 1) for case in pending))
            loop.run_until_complete(run())
            pending.clear()
            sys.stdout, sys.stderr = _outputs

    try:
        for item in request["tests"]:
            if _dirty:
                break
            case = Case(item, coverage)
            case.prepare()
            if case.errors or case.skip is not None:
                _send(case.result(bool(pending)))
            elif not case.unittest and (_is_async(case.target) or (case.instance is not None and any(_is_async(getattr(case.instance, name, None)) for name in ("setUp", "asyncSetUp", "asyncTearDown", "tearDown")))):
                if case.serial:
                    flush()
                pending.append(case)
                if case.serial or len(pending) >= concurrency:
                    flush()
            else:
                flush()
                if _dirty:
                    break
                _sync_case(case)
        flush()
    finally:
        if coverage:
            sys.settrace(None)


def main():
    if sys.version_info < (3, 12):
        _send({"startup_error": "Taut requires Python 3.12 or newer; select a supported interpreter with --python."})
        return
    if sys.flags.optimize:
        _send({"startup_error": "Python optimization disables test assertions. Remove PYTHONOPTIMIZE and use an interpreter without -O/-OO."})
        return
    _send({"ready": True, "python_version": ".".join(map(str, sys.version_info[:3]))})
    while True:
        request = _read()
        if request is None or request.get("cmd") == "shutdown":
            return
        _run_batch(request)
        _send({"done": True, "restart": _dirty})
        if _dirty:
            return


if __name__ == "__main__":
    main()
