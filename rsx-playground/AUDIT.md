# Playground Audit Checklist

## Dashboard Pages Render
- [ ] 1. Root page returns HTML — test: `api_e2e_test.py::test_root_returns_html`
- [ ] 2. Overview page renders — test: `api_e2e_test.py::test_overview_page`
- [ ] 3. Topology page renders — test: `api_e2e_test.py::test_topology_page`
- [ ] 4. Book page renders — test: `api_e2e_test.py::test_book_page`
- [ ] 5. Risk page renders with partials — test: `api_e2e_test.py::test_risk_page`
- [ ] 6. WAL page renders — test: `api_e2e_test.py::test_wal_page`
- [ ] 7. Orders page displays form — test: `api_orders_test.py::test_orders_page_displays_form`
- [ ] 8. Docs pages render with sidebar/tabs — test: `api_e2e_test.py::test_docs_has_sidebar`

## API Endpoints (REST)
- [ ] 9. /api/processes returns JSON list — test: `api_e2e_test.py::test_api_processes_returns_json_list`
- [ ] 10. /api/scenarios returns list including minimal — test: `api_e2e_test.py::test_api_scenarios_returns_json_list`
- [ ] 11. /api/logs returns JSON with filters — test: `api_logs_metrics_test.py::test_logs_combined_filters`
- [ ] 12. /api/stats returns JSON — test: `api_e2e_test.py::test_api_orders_stress_post`
- [ ] 13. /healthz returns JSON with gateway field — test: `api_e2e_test.py::test_healthz_returns_json`
- [ ] 14. /api/latency endpoint tracks sim-mode latency — test: `api_logs_metrics_test.py::test_metrics_latency_tracking`
- [ ] 15. /api/mark/prices returns sim prices when offline — test: `api_integration_test.py::test_mark_prices_returns_sim_when_offline`

## Process Management
- [ ] 16. Start all processes for minimal scenario — test: `api_processes_test.py::test_start_all_minimal_scenario`
- [ ] 17. Stop all processes clears managed dict — test: `api_processes_test.py::test_stop_all_stops_managed_processes`
- [ ] 18. Restart process changes PID — test: `api_processes_test.py::test_restart_changes_pid`
- [ ] 19. Kill process removes PID file — test: `api_processes_test.py::test_kill_removes_pid_file`
- [ ] 20. Build failure prevents start — test: `api_processes_test.py::test_build_failure_prevents_start`

## Order Flow and Data
- [ ] 21. Submit test order via form, appears in recent — test: `api_orders_test.py::test_orders_test_accepted_response_stored`
- [ ] 22. Batch orders submit 10, alternate sides — test: `api_orders_test.py::test_orders_batch_orders_alternate_sides`
- [ ] 23. Random orders submit 5 with variety — test: `api_orders_test.py::test_orders_random_orders_have_valid_symbols`
- [ ] 24. Cancel order updates status in place — test: `api_orders_test.py::test_orders_cancel_updates_status_in_place`
- [ ] 25. Invalid order marked rejected — test: `api_orders_test.py::test_orders_invalid_appends_rejected_order`

## Sim Mode (Gateway Offline)
- [ ] 26. Order submission queues when gateway down — test: `api_orders_test.py::test_orders_test_gateway_down_queues_order`
- [ ] 27. Sim book seeded with realistic BBO — test: `api_integration_test.py::test_book_endpoint_no_processes_returns_200`
- [ ] 28. Stress run returns error when gateway down — test: `test_stress_api.py::test_stress_run_gateway_down_returns_502`

## Trade UI (/v1 Endpoints)
- [ ] 29. /v1/symbols returns all configured symbols — test: `api_proxy_test.py::test_v1_symbols_returns_200`
- [ ] 30. /v1/account returns user balance/equity — test: `api_proxy_test.py::test_v1_proxy_502_not_500_on_connection_refused`
- [ ] 31. /v1/orders returns user open orders — `api_e2e_test.py::test_v1_orders_returns_list`
- [ ] 32. /v1/candles returns OHLCV data — `api_e2e_test.py::test_v1_candles_returns_bars`

## WAL Inspection
- [ ] 33. WAL status shows file count and size — test: `api_wal_test.py::test_wal_status_shows_file_count`
- [ ] 34. WAL dump lists files and events — test: `api_wal_test.py::test_wal_dump_shows_events`
- [ ] 35. WAL verify counts files per stream — test: `api_wal_test.py::test_wal_verify_counts_files`

## Error Handling and Edge Cases
- [ ] 36. Path traversal in WAL stream blocked — test: `api_edge_cases_test.py::test_path_traversal_in_wal_stream`
- [ ] 37. XSS prevention in order fields — test: `api_edge_cases_test.py::test_xss_in_cid`
- [ ] 38. SQL injection in user_id blocked — test: `api_edge_cases_test.py::test_sql_injection_in_user_id`

## Performance
- [ ] 39. Stress test 100 orders with latency tracking — test: `stress_integration_test.py::test_stress_high_100_orders_per_sec_60s`

## Security
- [ ] 40. Production mode refuses to start — `api_e2e_test.py::test_production_mode_guard`
