# PLAN

## goal
Fix multi-symbol process spawning so all RSX scenarios spawn the correct
processes with correct per-symbol routing.

## approach
Three independent changes: (1) rsx-risk reads `RSX_ME_CMP_ADDRS` and
routes outbound orders to the correct ME by symbol_id; (2) rsx-marketdata
reads `RSX_ME_CMP_ADDRS` and subscribes to all MEs; (3) the `start` script
computes `me_cmp_addrs`, passes them to Risk/Marketdata, fixes the Mark
Binance URL, and adds the Recorder process. Tasks 1 and 2 are independent
Rust crate changes; task 3 is a Python script change that depends on
knowing the env var names from tasks 1-2 (already specified in the design).

## tasks
- [ ] Fix rsx-risk: read RSX_ME_CMP_ADDRS, route by symbol_id
- [ ] Fix rsx-marketdata: read RSX_ME_CMP_ADDRS, subscribe all MEs
- [ ] Fix start script: env vars, Mark URL, add Recorder entry
