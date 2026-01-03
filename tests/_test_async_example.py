import asyncio


def test_sync_ok():
    assert 1 + 1 == 2


async def test_async_ok():
    await asyncio.sleep(0)
    assert True
