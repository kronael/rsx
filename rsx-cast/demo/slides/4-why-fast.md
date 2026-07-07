## why it's fast

WAL append is batched, off
the critical send path.

- ties raw UDP
- beats TCP and QUIC
