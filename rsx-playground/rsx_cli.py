"""Glue to the rsx-cli binary: finite JSON calls and long-lived children.

The binary is target/debug/rsx-cli; a missing binary is an error, never a
fallback. Every child starts in its own session, runs inside `spawn` and is
stopped there by PID (SIGTERM, a grace period, SIGKILL). On Linux the kernel
also SIGKILLs a child when the thread that started it dies, so a playground
killed without cleanup leaves none behind. `reap_stale` at the next start is
the fallback where that is unavailable.
"""

import asyncio
import ctypes
import json
import logging
import os
import signal
import sys
from collections.abc import AsyncIterator
from collections.abc import Callable
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

log = logging.getLogger(__name__)

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "debug" / "rsx-cli"
PID_DIR = ROOT / "tmp" / "cli-pids"
RUN_TIMEOUT_S = 10.0
STOP_GRACE_S = 3.0
PR_SET_PDEATHSIG = 1
DEFAULT_LINE_LIMIT = 1 << 16


def die_with_parent() -> Callable[[], None] | None:
    """A `preexec_fn` that makes the kernel SIGKILL the child with its parent.

    PR_SET_PDEATHSIG follows the thread that forked the child, which is the
    event loop's thread, and survives setsid and exec. The child exits when
    the parent is already gone, since the signal only covers a later death.
    None off Linux.
    """
    if sys.platform != "linux":
        return None
    prctl = ctypes.CDLL(None, use_errno=True).prctl
    parent_pid = os.getpid()

    def set_parent_death_signal() -> None:
        if prctl(PR_SET_PDEATHSIG, signal.SIGKILL) != 0 or os.getppid() != parent_pid:
            os._exit(1)

    return set_parent_death_signal


class CliError(Exception):
    def __init__(self, message: str, code: int | None = None, stdout: str = "") -> None:
        super().__init__(message)
        self.code = code
        self.stdout = stdout


def error_message(stderr: str) -> str:
    """The `error` of the last `{"error": ...}` stderr line, else the text."""
    for line in reversed(stderr.strip().splitlines()):
        try:
            return str(json.loads(line)["error"])
        except (ValueError, KeyError, TypeError):
            continue
    return stderr.strip() or "no error output"


async def stop(proc: asyncio.subprocess.Process) -> None:
    if proc.returncode is not None:
        return
    proc.terminate()
    try:
        await asyncio.wait_for(proc.wait(), STOP_GRACE_S)
    except TimeoutError:
        proc.kill()
        await proc.wait()


@asynccontextmanager
async def spawn(
    *args: str,
    pid_file: Path | None = None,
    has_stdin: bool = False,
    line_limit: int = DEFAULT_LINE_LIMIT,
) -> AsyncIterator[asyncio.subprocess.Process]:
    try:
        proc = await asyncio.create_subprocess_exec(
            str(BINARY),
            *args,
            limit=line_limit,
            stdin=asyncio.subprocess.PIPE if has_stdin else asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            start_new_session=True,
            preexec_fn=die_with_parent(),
        )
    except OSError as err:
        raise CliError(f"cannot start {BINARY}: {err}") from err
    if pid_file is not None:
        pid_file.parent.mkdir(parents=True, exist_ok=True)
        pid_file.write_text(str(proc.pid))
    try:
        yield proc
    finally:
        await stop(proc)
        if pid_file is not None:
            pid_file.unlink(missing_ok=True)


async def run_text(
    *args: str, timeout: float = RUN_TIMEOUT_S, stdin: str | None = None
) -> str:
    async with spawn(*args, has_stdin=stdin is not None) as proc:
        try:
            out, err = await asyncio.wait_for(
                proc.communicate(None if stdin is None else stdin.encode()), timeout
            )
        except TimeoutError:
            command = " ".join(args)
            raise CliError(
                f"rsx-cli {command} timed out after {timeout:.0f}s"
            ) from None
    stdout = out.decode(errors="replace")
    if proc.returncode != 0:
        message = error_message(err.decode(errors="replace"))
        raise CliError(message, proc.returncode, stdout)
    return stdout


def parse_line(line: str) -> dict[str, Any]:
    try:
        parsed = json.loads(line)
    except ValueError as err:
        raise CliError(f"unparseable rsx-cli output: {err}: {line[:200]}") from err
    if not isinstance(parsed, dict):
        raise CliError(f"unexpected rsx-cli output: {line[:200]}")
    return parsed


async def run(*args: str, timeout: float = RUN_TIMEOUT_S) -> dict[str, Any]:
    """One JSON object from a finite `--json` command."""
    return parse_line(await run_text(*args, timeout=timeout))


async def run_lines(*args: str, timeout: float = RUN_TIMEOUT_S) -> list[dict[str, Any]]:
    """The JSON Lines of a finite streaming command, such as `wal dump`."""
    text = await run_text(*args, timeout=timeout)
    return [parse_line(line) for line in text.splitlines() if line.strip()]


async def check_binary() -> None:
    """Fails with CliError when the binary is missing or does not run."""
    await run_text("--version")


class Follow:
    """A running streaming child: its JSON Lines and its last stderr line."""

    def __init__(self, proc: asyncio.subprocess.Process) -> None:
        self.proc = proc
        self.pid = proc.pid
        self.stderr_tail = ""
        self.stderr_task = asyncio.create_task(self.read_stderr())

    async def read_stderr(self) -> None:
        assert self.proc.stderr is not None
        async for raw in self.proc.stderr:
            line = raw.decode(errors="replace").strip()
            if line:
                self.stderr_tail = line

    async def lines(self) -> AsyncIterator[dict[str, Any]]:
        assert self.proc.stdout is not None
        async for raw in self.proc.stdout:
            yield parse_line(raw.decode(errors="replace"))

    async def exit_error(self) -> CliError:
        """The error for a child whose stdout ended."""
        code = await self.proc.wait()
        await self.stderr_task
        return CliError(error_message(self.stderr_tail), code)


@asynccontextmanager
async def follow(
    *args: str, pid_file: Path | None = None, line_limit: int = DEFAULT_LINE_LIMIT
) -> AsyncIterator[Follow]:
    async with spawn(*args, pid_file=pid_file, line_limit=line_limit) as proc:
        child = Follow(proc)
        try:
            yield child
        finally:
            child.stderr_task.cancel()


def reap_stale(pid_dir: Path = PID_DIR) -> None:
    """SIGTERM the rsx-cli children a dead playground left behind.

    A pid file names a child this program started; the pid is signalled only
    while /proc still shows an rsx-cli command line for it.
    """
    if not pid_dir.exists():
        return
    for pid_file in pid_dir.glob("*.pid"):
        try:
            pid = int(pid_file.read_text())
            command = Path(f"/proc/{pid}/cmdline").read_bytes()
        except (OSError, ValueError):
            pid_file.unlink(missing_ok=True)
            continue
        if b"rsx-cli" in command:
            log.warning("rsx-cli child %d outlived its playground; stopping", pid)
            os.kill(pid, signal.SIGTERM)
        pid_file.unlink(missing_ok=True)
