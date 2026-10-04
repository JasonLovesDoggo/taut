"""Dependency-free Python execution worker. Test code never writes to the protocol fd."""
import contextvars
import importlib.util
import io
import json
import os
import struct
import sys
import tempfile
import time
import traceback
import types

_MAX_OUTPUT = 1024 * 1024
_protocol = os.fdopen(os.dup(1), "wb", buffering=0)
# dup() descriptors are non-inheritable: child processes cannot keep our protocol open.
_native = [tempfile.TemporaryFile(), tempfile.TemporaryFile()]
_native_offsets = [0, 0]
for _fd, _file in enumerate(_native, 1):
    os.dup2(_file.fileno(), _fd)
_capture = contextvars.ContextVar("taut_capture", default=None)
_owner = contextvars.ContextVar("taut_owner", default=None)
_modules = {}
_loop = None
_asyncio = None
_single_owner = None


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


def _send(value):
    data = json.dumps(value, ensure_ascii=True, separators=(",", ":")).encode()
    _protocol.write(struct.pack("<I", len(data)) + data)


def _read():
    header = sys.stdin.buffer.read(4)
    if not header:
        return None
    if len(header) != 4:
        raise EOFError("Incomplete request header")
    length = struct.unpack("<I", header)[0]
    if length > 64 * 1024 * 1024:
        raise ValueError("Request exceeds 64 MiB")
    data = sys.stdin.buffer.read(length)
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
            native = os.pread(_native[index].fileno(), min(size, _MAX_OUTPUT), _native_offsets[index]) if size else b""
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

        def factory(loop, coro, **kwargs):
            task = asyncio.Task(coro, loop=loop, **kwargs)
            context = kwargs.get("context")
            owner = context.get(_owner) if context is not None else _owner.get()
            if owner is not None:
                owner.tasks.add(task)
            return task

        _loop.set_task_factory(factory)
    return _loop


async def _call_async(function):
    value = _check_return(function())
    if hasattr(value, "__await__"):
        return _check_return(await value)
    return value


async def _cleanup_tasks(case):
    current = _asyncio.current_task()
    # Descendants may create descendants during cancellation, so keep draining.
    while True:
        tasks = [task for task in case.tasks if task is not current]
        case.tasks.difference_update(tasks)
        if not tasks:
            return
        for task in tasks:
            if not task.done():
                task.cancel()
        values = await _asyncio.gather(*tasks, return_exceptions=True)
        for value in values:
            if isinstance(value, BaseException) and not isinstance(value, _asyncio.CancelledError):
                case.record(value, "background task")


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


def _call_sync(function, case):
    value = _check_return(function())
    if hasattr(value, "__await__"):
        loop = _ensure_loop()
        async def await_value():
            return _check_return(await value)
        return loop.run_until_complete(await_value())
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
        if case.tasks:
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


if __name__ == "__main__":
    main()
