#!/usr/bin/env python3
"""Test all page routes return HTTP 200."""

import asyncio
import sys

import aiohttp


ROUTES = [
    "/",
    "/overview",
    "/topology",
    "/book",
    "/risk",
    "/wal",
    "/logs",
    "/control",
    "/faults",
    "/verify",
    "/orders",
    "/stress",
    "/docs",
]

BASE_URL = "http://localhost:49171"


async def test_route(session: aiohttp.ClientSession, route: str):
    url = f"{BASE_URL}{route}"
    try:
        async with session.get(url, timeout=aiohttp.ClientTimeout(total=5)) as resp:
            status = resp.status
            if status == 200:
                print(f"✓ {route:20s} HTTP {status}")
                return True
            else:
                print(f"✗ {route:20s} HTTP {status}")
                return False
    except Exception as e:
        print(f"✗ {route:20s} ERROR: {e}")
        return False


async def main():
    async with aiohttp.ClientSession() as session:
        results = await asyncio.gather(
            *[test_route(session, route) for route in ROUTES]
        )

    passed = sum(results)
    total = len(results)

    print(f"\nResults: {passed}/{total} routes returned HTTP 200")

    if passed == total:
        print("✓ All routes passed")
        sys.exit(0)
    else:
        print(f"✗ {total - passed} routes failed")
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
