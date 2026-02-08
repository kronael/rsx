# Critique (Deep Gaps from Specs)

All previously listed gaps have been resolved.

Remaining risks are accepted tradeoffs:
- Ingress orders can be lost (best‑effort ACK).
- Backpressure correctness depends on strict stalling.
- UTC scheduling depends on clock sync.
