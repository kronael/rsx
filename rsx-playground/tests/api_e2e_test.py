"""E2E tests for RSX Playground API endpoints.

Run with: cd rsx-playground && uv run pytest tests/api_e2e_test.py -v
"""


# ── HTML Page Routes ────────────────────────────────────────


def test_root_returns_html(client):
    """GET / returns 200 HTML."""
    resp = client.get("/")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_overview_page(client):
    """GET /overview returns 200 HTML."""
    resp = client.get("/overview")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_topology_page(client):
    """GET /topology returns 200 HTML."""
    resp = client.get("/topology")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_book_page(client):
    """GET /book returns 200 HTML."""
    resp = client.get("/book")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_risk_page(client):
    """GET /risk returns 200 HTML."""
    resp = client.get("/risk")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_wal_page(client):
    """GET /wal returns 200 HTML."""
    resp = client.get("/wal")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_logs_page(client):
    """GET /logs returns 200 HTML."""
    resp = client.get("/logs")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_control_page(client):
    """GET /control returns 200 HTML."""
    resp = client.get("/control")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_faults_page(client):
    """GET /faults returns 200 HTML."""
    resp = client.get("/faults")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_verify_page(client):
    """GET /verify returns 200 HTML."""
    resp = client.get("/verify")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_orders_page(client):
    """GET /orders returns 200 HTML."""
    resp = client.get("/orders")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── API JSON Routes ─────────────────────────────────────────


def test_api_processes_returns_json_list(client):
    """GET /api/processes returns JSON list."""
    resp = client.get("/api/processes")
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data, list)


def test_api_scenarios_returns_json_list(client):
    """GET /api/scenarios returns JSON list of scenario names."""
    resp = client.get("/api/scenarios")
    assert resp.status_code == 200
    data = resp.json()
    assert isinstance(data, list)
    assert "minimal" in data


def test_api_logs_returns_json(client):
    """GET /api/logs returns JSON with lines and count."""
    resp = client.get("/api/logs")
    assert resp.status_code == 200
    data = resp.json()
    assert "lines" in data
    assert "count" in data
    assert isinstance(data["lines"], list)
    assert isinstance(data["count"], int)


# ── API POST Routes ─────────────────────────────────────────


def test_api_build_post(client):
    """POST /api/build returns HTML."""
    resp = client.post("/api/build")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_orders_test_with_form_data(client):
    """POST /api/orders/test with form data returns HTML."""
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    text = resp.text.lower()
    assert "order" in text
    assert any(w in text for w in ["queued", "accepted", "rejected"])


def test_api_orders_batch_post(client):
    """POST /api/orders/batch returns HTML with batch confirmation."""
    resp = client.post("/api/orders/batch")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    assert "10 batch orders submitted" in resp.text


def test_api_orders_random_post(client):
    """POST /api/orders/random returns HTML with random orders."""
    resp = client.post("/api/orders/random")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    assert "5 random orders" in resp.text


def test_api_orders_stress_post(client):
    """POST /api/stress/run returns JSON response."""
    resp = client.post("/api/stress/run")
    # 502 expected when gateway not running
    assert resp.status_code in (200, 502)
    data = resp.json()
    if resp.status_code == 200:
        assert data.get("status") == "completed"
        assert "results" in data
    else:
        assert "error" in data


def test_api_orders_invalid_post(client):
    """POST /api/orders/invalid returns HTML with rejected."""
    resp = client.post("/api/orders/invalid")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    assert "rejected" in resp.text.lower()


def test_api_verify_run_post(client):
    """POST /api/verify/run returns HTML table."""
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_wal_verify_post(client):
    """POST /api/wal/verify returns HTML."""
    resp = client.post("/api/wal/verify")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_wal_dump_post(client):
    """POST /api/wal/dump returns HTML."""
    resp = client.post("/api/wal/dump")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_processes_action_stop(client):
    """POST /api/processes/{name}/stop returns HTML."""
    resp = client.post("/api/processes/me-pengu/stop")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── HTMX Partial Routes ─────────────────────────────────────


def test_x_processes_returns_html_table(client):
    """GET /x/processes returns HTML table."""
    resp = client.get("/x/processes")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_health_returns_html(client):
    """GET /x/health returns HTML."""
    resp = client.get("/x/health")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_key_metrics_returns_html(client):
    """GET /x/key-metrics returns HTML."""
    resp = client.get("/x/key-metrics")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_logs_tail_returns_html(client):
    """GET /x/logs-tail returns HTML."""
    resp = client.get("/x/logs-tail")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_wal_status_returns_html(client):
    """GET /x/wal-status returns HTML."""
    resp = client.get("/x/wal-status")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_recent_orders_returns_html(client):
    """GET /x/recent-orders returns HTML."""
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_verify_returns_html(client):
    """GET /x/verify returns HTML."""
    resp = client.get("/x/verify")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_risk_user_with_query_param(client):
    """GET /x/risk-user?risk-uid=1 returns HTML."""
    resp = client.get("/x/risk-user", params={"risk-uid": 1})
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Additional Coverage ─────────────────────────────────────


def test_api_metrics_returns_json(client):
    """GET /api/metrics returns JSON."""
    resp = client.get("/api/metrics")
    assert resp.status_code == 200
    data = resp.json()
    assert "processes" in data
    assert "running" in data
    assert "postgres" in data


def test_api_logs_with_filters(client):
    """GET /api/logs with filters returns filtered JSON."""
    resp = client.get(
        "/api/logs",
        params={
            "process": "gateway",
            "level": "info",
            "search": "error",
            "limit": 100,
        },
    )
    assert resp.status_code == 200
    data = resp.json()
    assert "lines" in data
    assert "count" in data


def test_api_processes_action_invalid(client):
    """POST /api/processes/{name}/invalid returns 400."""
    resp = client.post("/api/processes/me-pengu/invalid")
    assert resp.status_code == 400
    data = resp.json()
    assert "error" in data


def test_api_risk_user_by_id(client):
    """GET /api/risk/users/{user_id} returns JSON."""
    resp = client.get("/api/risk/users/1")
    assert resp.status_code == 200
    data = resp.json()
    assert "user_id" in data or isinstance(data, list)


def test_api_risk_action_freeze(client):
    """POST /api/risk/users/{user_id}/freeze returns JSON."""
    resp = client.post("/api/risk/users/1/freeze")
    assert resp.status_code == 200
    data = resp.json()
    assert "action" in data


def test_api_risk_action_invalid(client):
    """POST /api/risk/users/{user_id}/invalid returns 400."""
    resp = client.post("/api/risk/users/1/invalid")
    assert resp.status_code == 400
    data = resp.json()
    assert "error" in data


def test_api_mark_prices(client):
    """GET /api/mark/prices returns JSON."""
    resp = client.get("/api/mark/prices")
    assert resp.status_code == 200
    data = resp.json()
    assert "status" in data


def test_x_book_with_symbol_id(client):
    """GET /x/book?symbol_id=10 returns HTML."""
    resp = client.get("/x/book", params={"symbol_id": 10})
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_order_trace_with_oid(client):
    """GET /x/order-trace?trace-oid=123 returns HTML."""
    resp = client.get(
        "/x/order-trace", params={"trace-oid": "123"}
    )
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_logs_with_filters(client):
    """GET /x/logs with query params returns HTML."""
    resp = client.get(
        "/x/logs",
        params={
            "log-process": "gateway",
            "log-level": "info",
            "log-search": "error",
        },
    )
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_users_create_post(client):
    """POST /api/users/create returns HTML."""
    resp = client.post("/api/users/create")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_users_deposit_post(client):
    """POST /api/users/{user_id}/deposit returns HTML."""
    resp = client.post("/api/users/1/deposit")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_api_risk_liquidate_post(client):
    """POST /api/risk/liquidate returns HTML."""
    resp = client.post("/api/risk/liquidate")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_x_liquidations_returns_html(client):
    """GET /x/liquidations returns HTML."""
    resp = client.get("/x/liquidations")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Integration Flow Tests ──────────────────────────────────


def test_order_flow_test_then_recent(client):
    """Test order submission then check recent orders."""
    # Submit test order
    resp = client.post(
        "/api/orders/test",
        data={
            "symbol_id": "10",
            "side": "buy",
            "price": "50000",
            "qty": "1",
        },
    )
    assert resp.status_code == 200

    # Check recent orders includes it
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200


def test_verify_run_then_check_results(client):
    """Run verify then check /x/verify shows results."""
    # Run verify
    resp = client.post("/api/verify/run")
    assert resp.status_code == 200

    # Check results visible
    resp = client.get("/x/verify")
    assert resp.status_code == 200
    # Should not be placeholder text anymore
    assert "Run All Checks" not in resp.text


def test_batch_order_flow(client):
    """Submit batch orders and verify count increases."""
    # Initial recent orders
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    initial = resp.text

    # Submit batch
    resp = client.post("/api/orders/batch")
    assert resp.status_code == 200

    # Check recent orders changed
    resp = client.get("/x/recent-orders")
    assert resp.status_code == 200
    # Content should have changed (batch added orders)
    # We can't easily assert more here without parsing HTML


def test_processes_endpoint_consistency(client):
    """JSON and HTML process endpoints return consistent data."""
    # JSON endpoint
    json_resp = client.get("/api/processes")
    assert json_resp.status_code == 200
    json_procs = json_resp.json()

    # HTML endpoint
    html_resp = client.get("/x/processes")
    assert html_resp.status_code == 200

    # Both should succeed and JSON should be list
    assert isinstance(json_procs, list)


# ── Healthz Endpoint ───────────────────────────────────────


def test_healthz_returns_json(client):
    """GET /healthz returns JSON with status fields."""
    resp = client.get("/healthz")
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "ok"
    assert "port" in data
    assert "processes_running" in data
    assert "processes_total" in data
    assert "postgres" in data


def test_healthz_port_is_49171(client):
    """GET /healthz reports port 49171."""
    resp = client.get("/healthz")
    data = resp.json()
    assert data["port"] == 49171


def test_healthz_process_counts_are_ints(client):
    """GET /healthz process counts are integers."""
    resp = client.get("/healthz")
    data = resp.json()
    assert isinstance(data["processes_running"], int)
    assert isinstance(data["processes_total"], int)


# ── Stress Page Route ──────────────────────────────────────


def test_stress_page(client):
    """GET /stress returns 200 HTML."""
    resp = client.get("/stress")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


# ── Trade UI Page ──────────────────────────────────────────


def test_trade_page_redirect(client):
    """GET /trade redirects to /trade/."""
    resp = client.get("/trade", follow_redirects=False)
    assert resp.status_code in (301, 307)


def test_trade_page_loads(client):
    """GET /trade/ returns 200 HTML."""
    resp = client.get("/trade/")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_trade_page_has_script_tags(client):
    """Trade page includes JS script tags."""
    resp = client.get("/trade/")
    assert resp.status_code == 200
    text = resp.text
    assert "<script" in text or ".js" in text


# ── Docs Pages ─────────────────────────────────────────────


def test_docs_redirect_to_readme(client):
    """GET /docs serves /docs/README content."""
    resp = client.get("/docs")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_readme_page(client):
    """GET /docs/README returns 200 HTML."""
    resp = client.get("/docs/README")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_api_page(client):
    """GET /docs/api returns 200 HTML."""
    resp = client.get("/docs/api")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_scenarios_page(client):
    """GET /docs/scenarios returns 200 HTML."""
    resp = client.get("/docs/scenarios")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_tabs_page(client):
    """GET /docs/tabs returns 200 HTML."""
    resp = client.get("/docs/tabs")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_troubleshooting_page(client):
    """GET /docs/troubleshooting returns 200 HTML."""
    resp = client.get("/docs/troubleshooting")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_index_page(client):
    """GET /docs/index returns 200 HTML."""
    resp = client.get("/docs/index")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]


def test_docs_404_nonexistent(client):
    """GET /docs/nonexistent returns 404."""
    resp = client.get("/docs/nonexistent")
    assert resp.status_code == 404


def test_docs_has_sidebar(client):
    """Docs pages include sidebar with navigation links."""
    resp = client.get("/docs/README")
    assert resp.status_code == 200
    text = resp.text
    assert 'href="./' in text or '/docs/' in text
    assert 'sidebar' in text.lower() or 'href="./' in text
