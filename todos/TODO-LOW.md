# TODO: Low Priority Bugs (5)

Optional, defensive. Latent issues, edge cases with no practical impact.

### [ ] Average entry price divide-by-zero (latent)
- **File:** rsx-risk/src/position.rs:105-114
- **Fix:** Add debug_assert!(self.long_qty > 0) or return 0.

### [ ] Postgres init race in start script
- **File:** start:511-541
- **Fix:** Add retry loop or pg_isready check before migration.

### [ ] System time panic before 1970
- **File:** rsx-types/src/time.rs:8,17,26,35
- **Fix:** Use .unwrap_or_default() instead of .unwrap().

### [ ] Hardcoded port 5432 in start script
- **File:** start:125-506
- **Fix:** Read from DATABASE_URL or add PORT config.

### [ ] Double stale update in mark aggregator
- **File:** rsx-mark/src/main.rs:~180
- **Fix:** Check stale flag before aggregate phase.
