"""CPU-intensive tests to demonstrate parallel execution."""

import math


def is_prime(n):
    if n < 2:
        return False
    for i in range(2, int(math.sqrt(n)) + 1):
        if n % i == 0:
            return False
    return True


def test_count_primes_to_50k():
    count = sum(1 for n in range(50000) if is_prime(n))
    assert count == 5133


def test_count_primes_to_60k():
    count = sum(1 for n in range(60000) if is_prime(n))
    assert count == 6057


def test_count_primes_to_70k():
    count = sum(1 for n in range(70000) if is_prime(n))
    assert count == 6935


def test_count_primes_to_80k():
    count = sum(1 for n in range(80000) if is_prime(n))
    assert count == 7837


def test_fibonacci_sum():
    def fib(n):
        a, b = 0, 1
        for _ in range(n):
            a, b = b, a + b
        return a

    total = sum(fib(i) for i in range(1000))
    assert total > 0


def test_matrix_multiply():
    size = 80
    a = [[i * j for j in range(size)] for i in range(size)]
    b = [[i + j for j in range(size)] for i in range(size)]

    result = [[sum(a[i][k] * b[k][j] for k in range(size)) for j in range(size)] for i in range(size)]
    assert len(result) == size


def test_string_processing():
    s = "abcdefghij" * 10000
    for _ in range(50):
        s = s.replace("abc", "xyz").replace("xyz", "abc")
    assert len(s) == 100000


def test_list_sorting():
    import random
    random.seed(42)
    for _ in range(100):
        data = [random.randint(0, 100000) for _ in range(5000)]
        sorted_data = sorted(data)
        assert sorted_data[0] <= sorted_data[-1]
