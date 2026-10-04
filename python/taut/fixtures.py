"""Small, dependency-free fixture declarations for taut."""


def fixture(func=None, *, name=None, autouse=False, scope="function", params=None):
    """Declare a per-test dependency, optionally using ``yield`` for cleanup.

    Sync functions, async functions, generators, and async generators are
    supported. Wider scopes and parametrized fixtures are not yet supported.
    """
    if scope != "function":
        raise ValueError("taut fixtures currently support only scope='function'")
    if params is not None:
        raise ValueError("taut does not yet support parametrized fixtures")
    if name is not None and (not isinstance(name, str) or not name.isidentifier()):
        raise ValueError("fixture name must be a Python identifier")

    def decorate(function):
        if not callable(function):
            raise TypeError("@fixture can only decorate a callable")
        function.__taut_fixture__ = {
            "name": name,
            "autouse": bool(autouse),
            "scope": scope,
            "params": params,
        }
        return function

    return decorate(func) if func is not None else decorate
