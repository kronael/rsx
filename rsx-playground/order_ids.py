"""Client order ids for in-repo order generators (spec 61, client rules).

A cid is 1-20 ASCII chars, never padded: a random session part drawn when
the generator starts, then a base36 counter. The gateway binds a cid to
its order per user for an hour and answers a repeat as DUPLICATE_ORDER_ID,
so a cid must not repeat within a run or across restarts.
"""

import itertools
import os
import secrets

ALPHABET = "0123456789abcdefghijklmnopqrstuvwxyz"
SESSION_CHARS = 8  # 36**8 is about 2**41: above the 40 random bits spec 61 asks


def base36(number: int) -> str:
    digits = ""
    while True:
        number, digit = divmod(number, 36)
        digits = ALPHABET[digit] + digits
        if number == 0:
            return digits


class CidMint:
    """The cids of one generator process: session part + base36 counter."""

    def __init__(self, session: str | None = None):
        self.session = session or "".join(
            secrets.choice(ALPHABET) for _ in range(SESSION_CHARS))
        self._counter = itertools.count(1)

    def next(self) -> str:
        return self.session + base36(next(self._counter))


# One mint per process, for callers that mint cids from many places. A
# forked child draws its own session: it must not repeat the parent's cids.
process_mint = CidMint()


def _renew_after_fork() -> None:
    global process_mint
    process_mint = CidMint()


os.register_at_fork(after_in_child=_renew_after_fork)


def next_cid(prefix: str = "") -> str:
    """A cid from the process mint; `prefix` labels the source in logs
    and counts toward the 20-char limit."""
    return prefix + process_mint.next()
