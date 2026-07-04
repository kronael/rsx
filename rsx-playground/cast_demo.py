"""RSX-CAST demonstration page for the playground dashboard.

Self-contained: renders a server-side HTML string (Tailwind CDN + a
little vanilla JS, no htmx required) that makes rsx-cast's advantages
visually apparent to a newcomer. Every number on the page is sourced
from the project's own READMEs / benches (see SOURCES below) — nothing
is invented. Bench vs live is labelled in the UI.

SOURCES (all repo docs, loopback p50 unless noted):
- rsx-cast/README.md "How fast" + compare/README.md  — protocol RTT table
- README.md (root) "Network stack — rsx-cast"        — wire microbenches
- rsx-cast/README.md "Wire format"                   — 16B header layout
- rsx-cast/README.md "Two-tier retransmit"           — ring -> WAL horizon
- specs/2/4-cast.md / CLAUDE.md "Trust boundaries"   — unauthenticated by design

Wire only with the snippet returned by the task; this module imports
nothing from server.py / pages.py so it stays importable on its own.
"""

# Match the dashboard's Tailwind source resolution without importing
# pages.py (kept import-free so this module compiles standalone). If a
# local CDN copy exists, use it; else fall back to the public CDN — same
# logic pages.py uses.
from pathlib import Path as _Path

_TAILWIND_LOCAL = _Path(__file__).resolve().parent / "tailwind-play.js"
_TAILWIND_SRC = (
    "./static/tailwind-play.js"
    if _TAILWIND_LOCAL.exists()
    else "https://cdn.tailwindcss.com"
)


def _card(title, body):
    return f"""
<div class="bg-slate-900 border border-slate-800 rounded-lg p-4">
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-xs font-semibold text-slate-500
      uppercase tracking-wider">{title}</h2>
  </div>
  {body}
</div>"""


# ── (1) The one-trick hero: wire == disk == replay ──────────────────

def _hero():
    return f"""
<div class="bg-gradient-to-br from-slate-900 to-slate-950
  border border-blue-900/50 rounded-lg p-5">
  <div class="text-blue-400 text-xs font-semibold uppercase
    tracking-wider mb-1">rsx-cast</div>
  <h1 class="text-xl font-bold text-white">
    The retransmit source <span class="text-blue-400">is</span>
    the WAL.</h1>
  <p class="text-sm text-slate-400 mt-2 max-w-3xl">
    The 16-byte header + <code class="bg-slate-800 px-1 rounded">
    repr(C)</code> payload the matching engine writes to its
    write-ahead log is the <em>same bytes</em> on the UDP wire and
    the <em>same bytes</em> on the TCP replay stream. No serialize
    step, no encode step, no length-prefix wrapper. Every other
    option puts a framing layer on top of records that are already
    framed; casting skips it.</p>

  <div class="grid grid-cols-1 md:grid-cols-3 gap-2 mt-4">
    <div class="bg-slate-800/60 rounded p-3 text-center">
      <div class="text-2xl">📡</div>
      <div class="text-xs text-slate-400 mt-1">UDP frame (live)</div>
    </div>
    <div class="bg-slate-800/60 rounded p-3 text-center">
      <div class="text-2xl">💾</div>
      <div class="text-xs text-slate-400 mt-1">WAL record (disk)</div>
    </div>
    <div class="bg-slate-800/60 rounded p-3 text-center">
      <div class="text-2xl">🔁</div>
      <div class="text-xs text-slate-400 mt-1">TCP replay (cold)</div>
    </div>
  </div>
  <div class="text-center text-emerald-400 text-sm font-bold mt-3">
    ↑ bitwise identical — one bytestream, three uses ↑</div>
</div>"""


# ── (2) Speed bars: casting vs the field (loopback p50, 128 B) ──────

# (label, p50_us, bench, color, note) — straight from
# rsx-cast/README.md "How fast" + compare/README.md.
_RTT = [
    ("casting (rsx-cast)", 10, "cast_rtt_bench", "emerald",
     "at the raw-UDP floor — protocol adds ~0 µs"),
    ("raw UDP (floor)", 10, "compare_all::raw_udp_128b", "slate",
     "sendto + recvfrom, no framing"),
    ("MoldUDP64 (Nasdaq)", 10, "compare_moldudp64", "slate",
     "UDP-sequenced frame + separate request server"),
    ("TCP_NODELAY", 14, "compare_all::tcp_nodelay_128b", "amber",
     "persistent conn, read_exact"),
    ("SoupBinTCP", 14, "compare_soupbintcp", "amber",
     "TCP + 3-byte framing"),
    ("KCP (turbo+spin)", 21, "compare_all::kcp_spin_flush_128b", "amber",
     "gaming RUDP, ARQ + congestion control"),
    ("Quinn / QUIC", 37, "compare_all::quinn_persistent_128b", "rose",
     "TLS 1.3 handshake + congestion-control state"),
    ("Aeron (UDP)", 305, "compare_aeron", "rose",
     "networked UDP path; 21 µs on a tuned AWS c6in"),
]
_RTT_MAX = 305.0


def _speed_bars():
    rows = ""
    for label, us, bench, color, note in _RTT:
        # log-ish scaling so 10 µs is visible next to 305 µs.
        pct = max(3.0, (us / _RTT_MAX) ** 0.45 * 100.0)
        bar = {
            "emerald": "bg-emerald-500",
            "slate": "bg-slate-500",
            "amber": "bg-amber-500",
            "rose": "bg-rose-500",
        }[color]
        txt = {
            "emerald": "text-emerald-400",
            "slate": "text-slate-300",
            "amber": "text-amber-400",
            "rose": "text-rose-400",
        }[color]
        rows += f"""
  <div class="grid grid-cols-12 items-center gap-2 py-1">
    <div class="col-span-4 text-xs {txt} truncate"
      title="{note}">{label}</div>
    <div class="col-span-6 bg-slate-800 rounded h-5 relative">
      <div class="{bar} h-5 rounded" style="width:{pct:.0f}%"></div>
    </div>
    <div class="col-span-2 text-right text-xs font-bold {txt}">
      ~{us} µs</div>
  </div>"""
    return _card(
        "Round-trip latency vs the field "
        "(loopback p50, 128 B = size_of FillRecord)",
        f"""<div class="space-y-0.5">{rows}</div>
<p class="text-xs text-slate-500 mt-3">
Bars use a compressed (≈√) scale so the 10 µs cluster stays
readable next to Aeron's 305 µs networked path. casting ties the
<span class="text-slate-300">raw-UDP floor</span> and
<span class="text-slate-300">MoldUDP64</span>, beats both TCP
protocols, both userspace-RUDP options (KCP, QUIC), and Aeron's
UDP path. The only faster numbers in the survey are shared-memory
<em>IPC</em> paths (Aeron IPC ~830 ns, Chronicle sub-µs) — not
network transports, and they carry no WAL.</p>
<p class="text-xs text-amber-400/80 mt-1">
Bench, not live: Criterion loopback microbenches, client+echoer
pinned to cores 2/3, Rust release. Reproduce:
<code class="bg-slate-800 px-1 rounded">cargo bench -p rsx-cast
--bench compare_all</code>. p50 only; p99 not yet measured. The
exchange's real cross-process p50 is ~1 128 µs — dominated by
runtime sleep-polls and PG churn, <em>not</em> transport (a ~10 µs
slice of that).</p>""",
    )


# ── (3) Where the 10 µs goes — send-path breakdown ──────────────────

def _breakdown():
    return _card(
        "Where the ~10 µs RTT actually goes",
        """<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
  <div>
    <div class="flex h-7 rounded overflow-hidden text-[10px]
      font-bold text-slate-900">
      <div class="bg-rose-400 flex items-center justify-center"
        style="width:99%" title="kernel sendto syscall">
        sendto syscall ~99%</div>
      <div class="bg-emerald-400 flex items-center
        justify-center" style="width:1%"
        title="casting protocol work">·</div>
    </div>
    <p class="text-xs text-slate-400 mt-2">
      <code class="bg-slate-800 px-1 rounded">CastSender::send</code>
      body is <span class="text-emerald-400 font-bold">~4.0 µs</span>,
      of which <span class="text-rose-400 font-bold">99% is the
      kernel <code>sendto</code></span>. The protocol's own work —
      assign seq, CRC32C, copy into the preallocated ring — rounds
      to zero. That is <em>why</em> casting sits at the raw-UDP
      floor: there is almost nothing on top of the syscall.</p>
  </div>
  <div class="text-xs space-y-1">
    <div class="flex justify-between border-b border-slate-800 py-1">
      <span class="text-slate-400">WAL append (in-memory)</span>
      <span class="text-emerald-400 font-bold">~31 ns</span></div>
    <div class="flex justify-between border-b border-slate-800 py-1">
      <span class="text-slate-400">Nak / Heartbeat encode</span>
      <span class="text-emerald-400 font-bold">~43 ns</span></div>
    <div class="flex justify-between border-b border-slate-800 py-1">
      <span class="text-slate-400">Fill encode</span>
      <span class="text-emerald-400 font-bold">~23 ns</span></div>
    <div class="flex justify-between border-b border-slate-800 py-1">
      <span class="text-slate-400">Fill decode</span>
      <span class="text-emerald-400 font-bold">~9 ns</span></div>
    <div class="flex justify-between py-1">
      <span class="text-slate-400">casting one-way (send→recv)</span>
      <span class="text-emerald-400 font-bold">~5.3 µs</span></div>
    <p class="text-amber-400/80 pt-1">Bench (release). Source:
      README.md "Network stack", rsx-cast/README.md "How fast".
      <span class="text-slate-300">Zero heap allocation on the send
      path</span> — every send is a copy into an already-owned ring
      slot + one syscall.</p>
  </div>
</div>""",
    )


# ── (4) The wire format — annotated 16-byte header ──────────────────

def _wire():
    cell = ("flex flex-col items-center justify-center border "
            "border-slate-700 bg-slate-800/60 rounded px-1 py-2")
    return _card(
        "Wire format = disk format (16-byte header, then repr(C) payload)",
        f"""<div class="flex flex-wrap gap-1 text-[10px] text-center">
  <div class="{cell}" style="flex:1 0 60px">
    <span class="text-blue-400 font-bold">version</span>
    <span class="text-slate-500">u8 · @0</span></div>
  <div class="{cell}" style="flex:1 0 50px">
    <span class="text-slate-400">_pad0</span>
    <span class="text-slate-500">u8 · @1</span></div>
  <div class="{cell}" style="flex:1 0 70px">
    <span class="text-emerald-400 font-bold">record_type</span>
    <span class="text-slate-500">u16 · @2</span></div>
  <div class="{cell}" style="flex:1 0 60px">
    <span class="text-emerald-400 font-bold">len</span>
    <span class="text-slate-500">u16 · @4</span></div>
  <div class="{cell}" style="flex:1 0 50px">
    <span class="text-slate-400">_pad1</span>
    <span class="text-slate-500">u16 · @6</span></div>
  <div class="{cell}" style="flex:1 0 80px">
    <span class="text-amber-400 font-bold">crc32c</span>
    <span class="text-slate-500">u32 · @8</span></div>
  <div class="{cell}" style="flex:1 0 70px">
    <span class="text-slate-400">reserved</span>
    <span class="text-slate-500">[u8;4] · @12</span></div>
</div>
<div class="flex gap-1 mt-1 text-[10px] text-center">
  <div class="{cell} w-full" style="background:rgba(59,130,246,0.08)">
    <span class="text-blue-300 font-bold">payload — repr(C, align(64)),
      ≤ 65535 B (seq = first u64 of every data record)</span></div>
</div>
<p class="text-xs text-slate-400 mt-3">
CRC32C (Castagnoli, SSE4.2 hardware path, ~1 cycle/8 B) over the
payload only. <span class="text-slate-300">version</span> at offset 0;
adding a new record type does <em>not</em> bump it. Little-endian,
compile-time enforced. Because there is no separate on-disk schema,
the byte you fsync is the byte you retransmit.</p>""",
    )


# ── (5) Two-tier retransmit — the embedded-WAL trick ────────────────

def _retransmit():
    return _card(
        "NAK retransmit, two tiers (embedded, not a sidecar)",
        """<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
  <div class="bg-slate-800/40 rounded p-3">
    <div class="text-xs text-slate-400 mb-2 font-mono">
      receiver detects a gap → sends Nak(seq)</div>
    <div class="space-y-2">
      <div class="border-2 border-emerald-500/70 rounded-[3px] px-3 py-1">
        <div class="text-emerald-400 text-xs font-bold">
          Tier 1 — hot ring (RAM)</div>
        <div class="text-xs text-slate-400">4 K most-recent frames,
          preallocated. <span class="text-emerald-400">µs to
          re-send.</span></div>
      </div>
      <div class="text-center text-slate-600 text-xs">↓ on miss</div>
      <div class="border-2 border-amber-500/70 rounded-[3px] px-3 py-1">
        <div class="text-amber-400 text-xs font-bold">
          Tier 2 — cold WAL (disk)</div>
        <div class="text-xs text-slate-400">pick the segment whose
          filename seq-range covers the target, scan it
          (<code class="bg-slate-900 px-1 rounded">read_record_at_seq</code>).
          <span class="text-amber-400">~10.4 ms @ 10 K records.</span></div>
      </div>
    </div>
    <div class="text-xs text-emerald-400 mt-3 font-bold">
      Retransmit horizon = WAL retention (4 h default), not RAM.</div>
  </div>
  <div class="text-xs text-slate-300 space-y-2 self-center">
    <p><span class="text-white font-bold">Aeron</span> ships its cold
    retransmit as a <em>separate Archive process</em>. casting keeps
    the audit log and the retransmit cache in the <em>same
    producer</em> — the protocol invention is small, the packaging
    difference is the point.</p>
    <p>A slow consumer never stalls the producer: it recovers via
    this NAK path, or — if it falls too far behind — escalates to a
    full TCP replay and resumes UDP. <span class="text-slate-400">
    Senders never pause; there is no flow control.</span></p>
    <p class="text-amber-400/80">10.4 ms cold-tier number is a
    bench (<code class="bg-slate-900 px-1 rounded">wal_random_read_bench</code>,
    real SSD); the 4 K / 4 h figures are config defaults.</p>
  </div>
</div>""",
    )


# ── (6) Packet-loss visual — TCP vs casting under a dropped frame ───

def _loss_visual():
    seq_box = ("inline-flex items-center justify-center w-7 h-7 "
               "rounded text-[10px] font-bold")
    return _card(
        "What happens when a packet drops (seq 3 lost)",
        f"""<div class="grid grid-cols-1 md:grid-cols-2 gap-4">
  <div>
    <div class="text-xs text-rose-400 font-bold mb-2">
      TCP — head-of-line blocking</div>
    <div class="flex gap-1 items-center flex-wrap">
      <span class="{seq_box} bg-emerald-600 text-white">1</span>
      <span class="{seq_box} bg-emerald-600 text-white">2</span>
      <span class="{seq_box} bg-rose-600 text-white">3</span>
      <span class="{seq_box} bg-slate-700 text-slate-500">4</span>
      <span class="{seq_box} bg-slate-700 text-slate-500">5</span>
    </div>
    <p class="text-xs text-slate-400 mt-2">
      4 and 5 already arrived but the kernel <em>withholds</em> them
      until 3 is re-ACKed and resent. The whole stream waits — even
      records that have nothing to do with the loss.</p>
  </div>
  <div>
    <div class="text-xs text-emerald-400 font-bold mb-2">
      casting — gap parked, NAK fills it</div>
    <div class="flex gap-1 items-center flex-wrap">
      <span class="{seq_box} bg-emerald-600 text-white">1</span>
      <span class="{seq_box} bg-emerald-600 text-white">2</span>
      <span class="{seq_box} bg-amber-500 text-slate-900">3?</span>
      <span class="{seq_box} bg-emerald-600 text-white">4</span>
      <span class="{seq_box} bg-emerald-600 text-white">5</span>
    </div>
    <p class="text-xs text-slate-400 mt-2">
      4 and 5 buffer in the reorder ring; the receiver
      <code class="bg-slate-900 px-1 rounded">Nak(3)</code>, the
      sender re-sends 3 from its hot ring (µs), delivery resumes in
      order. Producer never blocked. Tuned for ≤ 0.01% loss on a
      trusted LAN — on a lossy WAN use QUIC/KCP instead.</p>
  </div>
</div>""",
    )


# ── (7) Feature matrix — casting vs the field ───────────────────────

def _feature_matrix():
    # straight from rsx-cast/compare/README.md "Features"
    rows = [
        ("Retransmit source", "hot ring + WAL", "term buffers (RAM)",
         "separate request server", "in-flight window", True),
        ("Retransmit horizon", "WAL retention (4 h)", "~192 MB RAM",
         "server policy", "in-flight window", True),
        ("Durability", "wire = disk format", "separate Archive",
         "external", "none", True),
        ("Wire format", "16 B repr(C)", "32 B + term offsets",
         "20 B header", "variable QUIC frames", True),
        ("Serialization step", "none (repr(C))", "session framing",
         "external", "QUIC framing", True),
        ("Congestion control", "none (trusted LAN)", "none", "none",
         "yes (TLS+CC)", False),
        ("Encryption", "none — gateway/L3 owns it", "none", "none",
         "TLS 1.3", False),
        ("Language", "Rust", "Java + C++", "any (public spec)",
         "Rust", False),
    ]
    body = """<div class="overflow-x-auto"><table class="w-full text-xs">
  <thead class="text-slate-500">
    <tr>
      <th class="text-left py-1 pr-2"></th>
      <th class="text-left py-1 px-2 text-emerald-400">casting</th>
      <th class="text-left py-1 px-2">Aeron</th>
      <th class="text-left py-1 px-2">MoldUDP64</th>
      <th class="text-left py-1 px-2">Quinn / QUIC</th>
    </tr>
  </thead>
  <tbody>"""
    for prop, cast, aeron, mold, quic, cast_wins in rows:
        cast_cls = ("px-2 py-1 text-emerald-400 font-semibold"
                    if cast_wins else "px-2 py-1 text-slate-300")
        body += f"""
    <tr class="border-t border-slate-800">
      <td class="py-1 pr-2 text-slate-400">{prop}</td>
      <td class="{cast_cls}">{cast}</td>
      <td class="px-2 py-1 text-slate-500">{aeron}</td>
      <td class="px-2 py-1 text-slate-500">{mold}</td>
      <td class="px-2 py-1 text-slate-500">{quic}</td>
    </tr>"""
    body += """
  </tbody></table></div>
<p class="text-xs text-slate-500 mt-3">
Source: <code class="bg-slate-800 px-1 rounded">
rsx-cast/compare/README.md</code>. "none" is often a
<em>feature</em> here: no congestion control and no encryption
are deliberate — casting is a trusted-LAN transport
(<code class="bg-slate-800 px-1 rounded">specs/2/4-cast.md
§10.4</code>) and delegates auth to the gateway (JWT/TLS) and
the L3 network. For the public internet, the project says use
QUIC — that is the right tool for that job, not casting.</p>"""
    return _card("casting vs the field — features", body)


# ── (8) When NOT to use it (honesty, owner stresses honest labelling) ─

def _when_not():
    items = [
        ("Public internet", "no TLS, no congestion control → use QUIC"),
        ("Lossy / high-jitter WAN", "NAK storms collapse throughput; "
         "tuned for ≤ 0.01% loss → use KCP"),
        ("Schema that changes often", "wire = disk = repr(C) means "
         "changes are stop-redeploy events → use protobuf / FlatBuffers"),
        ("Multi-language consumers", "no IDL; hand-write the repr(C) "
         "layout per language"),
        ("One-to-many fan-out today", "single sender per stream; "
         "v2 multicast is specced, not shipped"),
        ("Latency-sensitive cold replay", "read_record_at_seq is O(N) "
         "within a segment (~10.4 ms @ 10 K)"),
    ]
    li = ""
    for head, body in items:
        li += f"""
    <li class="flex gap-2">
      <span class="text-rose-400">✗</span>
      <span><span class="text-slate-300 font-semibold">{head}</span>
        <span class="text-slate-500"> — {body}</span></span></li>"""
    return _card(
        "When NOT to use casting (it is not fastest-in-general)",
        f"""<ul class="space-y-1.5 text-xs">{li}</ul>
<p class="text-xs text-slate-500 mt-3">
casting is "at the UDP floor for a fixed-record trusted-LAN
workload", not a universal winner — and the project says so out
loud. Source: <code class="bg-slate-800 px-1 rounded">
rsx-cast/README.md</code> "When NOT to use this".</p>""",
    )


def cast_page() -> str:
    """Full standalone HTML for the rsx-cast demonstration page."""
    nav = """
<nav class="flex flex-wrap items-center gap-1 px-2 sm:px-4 py-2
  bg-slate-900 border-b border-slate-800">
  <span class="text-sm font-bold text-blue-400 mr-4
    tracking-wider">RSX</span>
  <a href="./overview" class="text-slate-400 hover:text-white
    hover:bg-slate-700 px-3 py-1.5 rounded text-xs font-mono">
    ← Dashboard</a>
  <span class="bg-slate-700 text-white px-3 py-1.5 rounded
    text-xs font-mono">Cast</span>
</nav>"""
    content = f"""
{_hero()}
{_speed_bars()}
<div class="grid grid-cols-1 lg:grid-cols-2 gap-3">
{_breakdown()}
{_wire()}
</div>
{_retransmit()}
{_loss_visual()}
{_feature_matrix()}
{_when_not()}"""
    return f"""<!DOCTYPE html>
<html lang="en" class="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>RSX -- Cast</title>
<script src="{_TAILWIND_SRC}"></script>
<script>
tailwind.config = {{
  darkMode: 'class',
  theme: {{ extend: {{ fontFamily: {{
    mono: ['SF Mono', 'Cascadia Code', 'Fira Code',
      'ui-monospace', 'monospace'],
  }} }} }},
}}
</script>
<style type="text/tailwindcss">
  body {{ font-family: theme('fontFamily.mono'); }}
</style>
</head>
<body class="bg-slate-950 text-slate-300 min-h-screen text-[13px]">
{nav}
<main class="p-2 sm:p-4 max-w-7xl mx-auto space-y-3">
{content}
</main>
<footer class="mt-8 py-4 px-4 border-t border-slate-800
  bg-slate-900 text-center text-xs text-slate-500">
  Every number sourced from repo docs / benches —
  loopback p50, Criterion release. Reproduce:
  <code class="bg-slate-800 px-1 rounded">cargo bench -p rsx-cast
  --bench compare_all</code>
</footer>
</body>
</html>"""


if __name__ == "__main__":
    print(cast_page())
