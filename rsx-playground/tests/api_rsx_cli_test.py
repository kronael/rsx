# pyright: basic
"""rsx-cli children die with the playground that started them.

Run with: cd rsx-playground && uv run pytest tests/api_rsx_cli_test.py -v
"""

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest

PLAYGROUND_DIR = Path(__file__).resolve().parent.parent

PARENT = """
import asyncio
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[1])
import rsx_cli

rsx_cli.BINARY = Path(sys.argv[2])


async def main():
    async with rsx_cli.spawn("follow") as proc:
        print(proc.pid, flush=True)
        await asyncio.sleep(300)


asyncio.run(main())
"""

FAKE_CLI = "#!/bin/sh\nexec sleep 300\n"


def is_dead(pid):
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return True
    return stat.rsplit(")", 1)[1].split()[0] == "Z"


def wait_until(condition, what, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


@pytest.mark.skipif(sys.platform != "linux", reason="PR_SET_PDEATHSIG is Linux")
def test_a_sigkilled_parent_takes_its_children_with_it(tmp_path):
    parent_script = tmp_path / "parent.py"
    parent_script.write_text(PARENT)
    fake_cli = tmp_path / "rsx-cli"
    fake_cli.write_text(FAKE_CLI)
    fake_cli.chmod(0o755)

    parent = subprocess.Popen(
        [sys.executable, "-I", str(parent_script), str(PLAYGROUND_DIR), str(fake_cli)],
        stdout=subprocess.PIPE,
        text=True,
    )
    child = None
    try:
        child = int(parent.stdout.readline())
        assert not is_dead(child)

        os.kill(parent.pid, signal.SIGKILL)
        parent.wait()

        wait_until(lambda: is_dead(child), "the child to die with its parent")
    finally:
        parent.stdout.close()
        if child is not None and not is_dead(child):
            os.kill(child, signal.SIGKILL)
