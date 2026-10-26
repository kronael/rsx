"""WAL views over rsx-cli.

Finite views (stream listing, verify, a one-off dump) are `rsx-cli wal ...
--json` calls. Live views read `WalFeed`: one `wal dump --follow --json`
child per stream, started and stopped with the playground, restarted with
backoff when it exits, its last error kept for the UI. The playground holds
no WAL decoder; a record is the CLI's JSON object with the type lowercased
and the order id named `order_id`.
"""

import asyncio
import heapq
import logging
import time
from collections import deque
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import rsx_cli

log = logging.getLogger(__name__)

KEPT_PER_TYPE = 5000
KEPT_FILLS = 20000
START_WINDOW_SEQS = 100000
BACKOFF_S = (0.5, 1.0, 2.0, 5.0, 10.0)
HEALTHY_RUN_S = 30.0
YIELD_EVERY = 500


@dataclass(frozen=True)
class Segment:
    name: str
    bytes: int
    modified_ms: int


@dataclass(frozen=True)
class Stream:
    label: str
    wal_dir: Path
    stream_id: int
    segments: list[Segment]

    def byte_count(self) -> int:
        return sum(segment.bytes for segment in self.segments)


@dataclass(frozen=True)
class Listing:
    name: str
    streams: list[Stream]


@dataclass
class Follower:
    stream: Stream
    next_seq: int
    task: asyncio.Task[None] | None = None
    pid: int | None = None
    restarts: int = 0
    records: int = 0
    error: str = ""
    warning: str = ""


def to_stream(label: str, wal_dir: Path, entry: dict[str, Any]) -> Stream:
    segments = [
        Segment(seg["name"], seg["bytes"], seg["modified_ms"])
        for seg in entry["segments"]
    ]
    return Stream(
        label=label,
        wal_dir=wal_dir,
        stream_id=entry["stream_id"],
        segments=segments,
    )


async def list_streams(wal_dir: Path) -> list[Listing]:
    """The streams under `wal_dir`, grouped by top-level directory.

    A numeric directory is its own stream; any other (an ME's `pengu`, the
    `mark` aggregator) groups the numeric stream directories inside it.
    Recorder archives are not listed. Sequence numbers are left out (`wal ls
    --no-seq`): finding them reads the last segment whole.
    """
    if not wal_dir.is_dir():
        return []
    names = sorted(
        entry.name
        for entry in wal_dir.iterdir()
        if entry.is_dir() and entry.name != "archive"
    )
    jobs: list[tuple[str | None, Path]] = []
    if any(name.isdigit() for name in names):
        jobs.append((None, wal_dir))
    for name in names:
        if not name.isdigit():
            jobs.append((name, wal_dir / name))
    reports = await asyncio.gather(
        *[
            rsx_cli.run("wal", "ls", str(directory), "--json", "--no-seq")
            for _, directory in jobs
        ]
    )
    listings: list[Listing] = []
    for (name, directory), report in zip(jobs, reports):
        if name is None:
            for entry in report["streams"]:
                label = str(entry["stream_id"])
                stream = to_stream(label, directory, entry)
                listings.append(Listing(label, [stream]))
        else:
            streams = [to_stream(name, directory, e) for e in report["streams"]]
            listings.append(Listing(name, streams))
    return sorted(listings, key=lambda listing: listing.name)


async def list_record_types() -> list[str]:
    """The lowercase record type names `rsx-cli wal types` knows."""
    report = await rsx_cli.run("wal", "types", "--json")
    return [name.lower() for name in report["types"]]


async def verify(listings: list[Listing]) -> dict[str, Any]:
    """`wal verify --json` over every directory that holds streams.

    A corrupt WAL exits 1 with the full report on stdout; that report is
    returned, any other failure raises CliError.
    """
    directories = sorted({s.wal_dir for listing in listings for s in listing.streams})
    streams: list[dict[str, Any]] = []
    for directory in directories:
        try:
            report = await rsx_cli.run("wal", "verify", str(directory), "--json")
        except rsx_cli.CliError as err:
            if not err.stdout:
                raise
            report = rsx_cli.parse_line(err.stdout)
        for entry in report["streams"]:
            streams.append({**entry, "wal_dir": str(directory)})
    corrupt = any(
        entry["crc_errors"] or entry["seq_gaps"] or entry["error"] for entry in streams
    )
    return {"status": "corrupt" if corrupt else "ok", "streams": streams}


def describe_problems(report: dict[str, Any], listings: list[Listing]) -> list[str]:
    """One line per corrupt stream of a `verify` report."""
    labels = {
        (str(s.wal_dir), s.stream_id): f"{s.label}/{s.stream_id}"
        for listing in listings
        for s in listing.streams
    }
    lines = []
    for entry in report["streams"]:
        parts = []
        if entry["crc_errors"]:
            parts.append(f"{entry['crc_errors']} crc errors")
        for first, last in entry["seq_gaps"]:
            parts.append(f"seq gap {first}-{last}")
        if entry["error"]:
            parts.append(entry["error"])
        if parts:
            key = (entry["wal_dir"], entry["stream_id"])
            lines.append(f"{labels.get(key, entry['stream_id'])}: {', '.join(parts)}")
    return lines


class WalFeed:
    def __init__(self, reject_reasons: dict[int, str]) -> None:
        self.reject_reasons = reject_reasons
        self.wal_dir: Path | None = None
        self.listings: list[Listing] = []
        self.refresh_error = ""
        self.followers: dict[tuple[Path, int], Follower] = {}
        self.recent: dict[str, deque[dict[str, Any]]] = {}
        self.seen: dict[tuple[str, str], int] = {}
        self.latest_bbo: dict[int, dict[str, Any]] = {}
        self.latest_mark: dict[int, dict[str, Any]] = {}

    def to_record(self, line: dict[str, Any], label: str) -> dict[str, Any]:
        record = {k: v for k, v in line.items() if k not in ("len", "crc32")}
        record["type"] = str(line["type"]).lower()
        record["stream"] = label
        if "oid" in record:
            record["order_id"] = record.pop("oid")
        if record["type"] == "order_failed":
            code = record["reason"]
            record["reason_text"] = self.reject_reasons.get(code, f"unknown_{code}")
        return record

    async def dump(self, wal_dir: Path) -> list[dict[str, Any]]:
        """Every record of every stream under `wal_dir`, once."""
        records: list[dict[str, Any]] = []
        for listing in await list_streams(wal_dir):
            for stream in listing.streams:
                lines = await rsx_cli.run_lines(
                    "wal", "dump", str(stream.stream_id), str(stream.wal_dir), "--json"
                )
                records.extend(self.to_record(line, listing.name) for line in lines)
        return records

    async def refresh(self, wal_dir: Path) -> None:
        """Follow every stream under `wal_dir` that is not followed yet.

        A failed listing is kept in `refresh_error` for the UI and retried
        on the next call. A stream that vanished or shrank means the WAL
        was wiped: the cache is dropped and rebuilt.
        """
        try:
            await self.do_refresh(wal_dir)
        except rsx_cli.CliError as err:
            self.refresh_error = str(err)
            log.warning("wal stream discovery failed: %s", err)
        else:
            self.refresh_error = ""

    async def do_refresh(self, wal_dir: Path) -> None:
        if self.wal_dir != wal_dir:
            await self.reset()
            self.wal_dir = wal_dir
        listings = await list_streams(wal_dir)
        self.listings = listings
        streams = {
            (s.wal_dir, s.stream_id): s for listing in listings for s in listing.streams
        }
        for key, follower in self.followers.items():
            stream = streams.get(key)
            if stream is None or stream.byte_count() < follower.stream.byte_count():
                log.warning("wal %s was wiped; following from scratch", key[0])
                await self.reset()
                self.wal_dir = wal_dir
                break
        for key, stream in streams.items():
            follower = self.followers.get(key)
            if follower is None:
                await self.start_follower(key, stream)
            else:
                follower.stream = stream

    async def start_follower(self, key: tuple[Path, int], stream: Stream) -> None:
        report = await rsx_cli.run(
            "wal", "ls", str(stream.wal_dir), "-s", str(stream.stream_id), "--json"
        )
        first_seq = report["streams"][0]["first_seq"]
        last_seq = report["streams"][0]["last_seq"]
        from_seq = 0
        if last_seq is not None:
            from_seq = max(first_seq or 0, last_seq - START_WINDOW_SEQS + 1)
        follower = Follower(stream, from_seq)
        follower.task = asyncio.create_task(self.follow_loop(follower))
        self.followers[key] = follower

    async def reset(self) -> None:
        tasks = [f.task for f in self.followers.values() if f.task is not None]
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        self.followers.clear()
        self.listings = []
        self.recent.clear()
        self.seen.clear()
        self.latest_bbo.clear()
        self.latest_mark.clear()

    async def follow_loop(self, follower: Follower) -> None:
        failures = 0
        while True:
            started = time.monotonic()
            try:
                await self.do_follow(follower)
            except rsx_cli.CliError as err:
                follower.error = str(err)
            except (KeyError, TypeError) as err:
                follower.error = f"unexpected record: {type(err).__name__}: {err}"
            follower.pid = None
            follower.restarts += 1
            lived_s = time.monotonic() - started
            failures = 1 if lived_s > HEALTHY_RUN_S else failures + 1
            delay = BACKOFF_S[min(failures, len(BACKOFF_S)) - 1]
            stream = follower.stream
            log.warning(
                "wal follower %s/%d: %s; restarting in %.1fs",
                stream.label,
                stream.stream_id,
                follower.error,
                delay,
            )
            await asyncio.sleep(delay)

    async def do_follow(self, follower: Follower) -> None:
        stream = follower.stream
        pid_file = rsx_cli.PID_DIR / f"wal-{stream.label}-{stream.stream_id}.pid"
        args = (
            "wal",
            "dump",
            str(stream.stream_id),
            str(stream.wal_dir),
            str(follower.next_seq),
            "--json",
            "--follow",
        )
        async with rsx_cli.follow(*args, pid_file=pid_file) as child:
            follower.pid = child.pid
            count = 0
            async for line in child.lines():
                self.ingest(follower, line)
                follower.warning = child.stderr_tail
                count += 1
                if count % YIELD_EVERY == 0:
                    await asyncio.sleep(0)
            error = await child.exit_error()
        if error.code == 0:
            raise rsx_cli.CliError("rsx-cli exited", 0)
        raise error

    def ingest(self, follower: Follower, line: dict[str, Any]) -> None:
        record = self.to_record(line, follower.stream.label)
        kind = record["type"]
        follower.next_seq = max(follower.next_seq, record["seq"] + 1)
        follower.records += 1
        kept = KEPT_FILLS if kind == "fill" else KEPT_PER_TYPE
        if kind not in self.recent:
            self.recent[kind] = deque(maxlen=kept)
        self.recent[kind].append(record)
        count_key = (record["stream"], kind)
        self.seen[count_key] = self.seen.get(count_key, 0) + 1
        if kind == "bbo":
            self.keep_newest(self.latest_bbo, record)
        elif kind == "mark_price":
            self.keep_newest(self.latest_mark, record)

    def keep_newest(
        self, latest: dict[int, dict[str, Any]], record: dict[str, Any]
    ) -> None:
        held = latest.get(record["symbol_id"])
        if held is None or record["seq"] > held["seq"]:
            latest[record["symbol_id"]] = record

    def records(self, kind: str | None = None) -> list[dict[str, Any]]:
        """The cached records of one type, or of every type."""
        if kind is not None:
            return list(self.recent.get(kind, ()))
        return [rec for held in list(self.recent.values()) for rec in list(held)]

    def newest(self, limit: int, kind: str | None = None) -> list[dict[str, Any]]:
        return heapq.nlargest(limit, self.records(kind), key=lambda r: r["seq"])

    def count_seen(self, kinds: set[str], skip_label: str) -> int:
        return sum(
            n
            for (label, kind), n in self.seen.items()
            if kind in kinds and label != skip_label
        )

    def fills(self, max_fills: int = 50) -> list[dict[str, Any]]:
        return self.newest(max_fills, "fill")

    def fills_for_user(
        self, user_id: int, symbol_id: int | None
    ) -> list[dict[str, Any]]:
        found = [
            fill
            for fill in self.records("fill")
            if user_id in (fill["taker_user_id"], fill["maker_user_id"])
            and (symbol_id is None or fill["symbol_id"] == symbol_id)
        ]
        return sorted(found, key=lambda fill: fill["seq"])

    def liquidations(self, user_id: int | None = None) -> list[dict[str, Any]]:
        found = [
            rec
            for rec in self.records("liquidation")
            if user_id is None or rec["user_id"] == user_id
        ]
        return sorted(found, key=lambda rec: rec["seq"], reverse=True)

    def bbo(self, symbol_id: int) -> dict[str, Any] | None:
        return self.latest_bbo.get(symbol_id)

    def book_stats(self) -> dict[int, dict[str, Any]]:
        return dict(self.latest_bbo)

    def mark_prices(self) -> dict[int, dict[str, Any]]:
        return dict(self.latest_mark)

    def follower_rows(self) -> list[dict[str, Any]]:
        return [
            {
                "stream": f"{f.stream.label}/{f.stream.stream_id}",
                "pid": f.pid,
                "records": f.records,
                "restarts": f.restarts,
                "error": f.error,
                "warning": f.warning,
            }
            for f in self.followers.values()
        ]

    def file_count(self) -> int:
        return sum(
            len(s.segments) for listing in self.listings for s in listing.streams
        )
