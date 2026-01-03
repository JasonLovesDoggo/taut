def test_math_is_broken():
    assert 1 + 1 == 3, "math stopped working"

def test_strings_are_weird():
    assert "hello" == "world"

def test_this_one_passes():
    assert True

def test_list_comparison():
    assert [1, 2, 3] == [1, 2, 4], "lists don't match"
