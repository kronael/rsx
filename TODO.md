# TODO

## Latency pipeline (from perf-verification.md)

- [ ] Fix play_latency tests: remove skip-on-404, assert
      p50 > 0
- [ ] Latency values always "-" in sim mode — need real
      gateway orders for measurement
- [ ] Stress reports show 0% accept rate, 0us p99 —
      orders go to sim fallback, not real gateway

## Scenarios not implemented

- [ ] "duo" scenario should start PENGU + SOL (2 MEs)
      but only starts same 3 as minimal
- [ ] "full" scenario should start PENGU + SOL + BTC
      with replicas — not implemented
- [ ] "stress" scenario — not implemented

## rsx-sim

- [ ] Stub only — load generator not implemented

## Trade UI

- [ ] Docs page 502 through nginx (works on direct port)
- [ ] No open positions display
- [ ] WS reconnect logic may be broken or proxy strips
      upgrade headers

## CLI

- [ ] RECORD_LIQUIDATION field decoding in rsx-cli
