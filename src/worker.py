"""Dependency-free Python execution worker. Test code never writes to the protocol fd."""
import contextvars
import importlib.util
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
    if owner is not None:
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


def _error(exc, stage):
    message = str(exc) or ("Assertion failed" if isinstance(exc, AssertionError) else type(exc).__name__)
    return {"message": stage + ": " + type(exc).__name__ + ": " + message,
            "traceback": "".join(traceback.format_exception(type(exc), exc, exc.__traceback__))}


def _is_skip(exc):
    return any(base.__name__ == "SkipTest" and base.__module__ == "unittest.case"
               for base in type(exc).__mro__)


def _check_return(value):
    if isinstance(value, (types.GeneratorType, types.AsyncGeneratorType)):
        if isinstance(value, types.GeneratorType):
            value.close()
        raise TypeError("Generator tests are unsupported; use a normal or async test function")
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
            for value in (module, cls, target):
                if getattr(value, "__unittest_skip__", False) or getattr(value, "_taut_skip", False):
                    self.skip = getattr(value, "__unittest_skip_why__", None) or getattr(value, "_taut_skip_reason", "")
                    return
            if cls is not None:
                if any(base.__name__ == "TestCase" and base.__module__ == "unittest.case" for base in cls.__mro__):
                    self.instance = cls(self.request["function"])
                else:
                    self.instance = cls()
                target = getattr(self.instance, self.request["function"])
            self.target = target
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
                     "traceback": "\n".join(e["traceback"] for e in self.errors)}
        result = {"id": self.request["id"], "passed": error is None,
                  "error": error, "skipped": self.skip is not None and error is None,
                  "skip_reason": self.skip, "stdout": output[0], "stderr": output[1],
                  "duration_sec": time.perf_counter() - self.start}
        if self.coverage is not None:
            result["coverage"] = {filename: sorted(lines) for filename, lines in self.coverage.items()}
        return result


def _ensure_loop():
    global _loop, _asyncio
    if _loop is None:
        import asyncio
        _asyncio = asyncio
        _loop = asyncio.new_event_loop()

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

        original_soon = _loop.call_soon
        original_at = _loop.call_at
        original_firstiter = _loop._asyncgen_firstiter_hook

        def call_soon(callback, *args, context=None):
            return original_soon(callback, *args, context=owned_context(callback, context))

        def call_at(when, callback, *args, context=None):
            return original_at(when, callback, *args, context=owned_context(callback, context))

        def firstiter(generator):
            original_firstiter(generator)
            owner = _owner.get()
            if owner is not None:
                owner.generators.add(generator)

        _loop.call_soon = call_soon
        _loop.call_at = call_at
        _loop._asyncgen_firstiter_hook = firstiter

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

        _loop.set_exception_handler(handle_error)
        _loop.set_task_factory(factory)
    return _loop


async def _call_async(function):
    value = _check_return(function())
    if hasattr(value, "__await__"):
        return _check_return(await value)
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
    try:
        if instance is not None and hasattr(instance, "setUp"):
            await _call_async(instance.setUp)
        setup_ok = True
        if instance is not None and hasattr(instance, "asyncSetUp"):
            await _call_async(instance.asyncSetUp)
        async_setup_ok = True
        await _call_async(case.target)
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


def _call_sync(function, case):
    value = _check_return(function())
    if hasattr(value, "__await__"):
        loop = _ensure_loop()
        async def await_value():
            return _check_return(await value)
        task = _asyncio.ensure_future(await_value(), loop=loop)
        try:
            return loop.run_until_complete(task)
        finally:
            case.tasks.discard(task)
    return value


def _sync_case(case):
    global _single_owner
    case.start = time.perf_counter()
    token = _capture.set(case.buffers)
    owner = _owner.set(case)
    _single_owner = case
    setup_ok = False
    try:
        if case.instance is not None and hasattr(case.instance, "setUp"):
            _call_sync(case.instance.setUp, case)
        setup_ok = True
        _call_sync(case.target, case)
    except BaseException as exc:
        case.record(exc, "test" if setup_ok else "setup")
    finally:
        if setup_ok and case.instance is not None and hasattr(case.instance, "tearDown"):
            try:
                _call_sync(case.instance.tearDown, case)
            except BaseException as exc:
                case.record(exc, "tearDown")
        if case.instance is not None:
            for cleanup, args, kwargs in reversed(getattr(case.instance, "_cleanups", [])):
                try:
                    _call_sync(lambda: cleanup(*args, **kwargs), case)
                except BaseException as exc:
                    case.record(exc, "cleanup")
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
            elif _is_async(case.target) or (case.instance is not None and any(_is_async(getattr(case.instance, name, None)) for name in ("setUp", "asyncSetUp", "asyncTearDown", "tearDown"))):
                pending.append(case)
                if len(pending) >= concurrency:
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
    _send({"ready": True})
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
