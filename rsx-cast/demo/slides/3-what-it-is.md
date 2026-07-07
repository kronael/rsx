## what it is

*peer-to-peer — not pub/sub,
not multicast*

- same bytes on wire and disk
  — batched WAL
- NAK recovers a live gap;
  replay is the fallback
  out-of-buffer
