"""Demos gallery API tests.

Run with: cd rsx-playground && uv run pytest tests/api_demos_test.py -v
"""

import pages


def test_demos_page_lists_every_wired_demo(client):
    """GET /demos renders the gallery referencing each wired demo GIF."""
    resp = client.get("/demos")
    assert resp.status_code == 200
    assert "text/html" in resp.headers["content-type"]
    body = resp.text
    assert "RSX Demos" in body
    wired = [k for k, c in pages.CRATES.items() if c.get("demo")]
    assert wired, "expected at least one crate with a demo wired"
    for key in wired:
        assert f"x/crate-demo/{key}" in body


def test_every_wired_demo_gif_serves_locally(client):
    """Each wired crate's demo GIF serves as image/gif with content."""
    for key, crate in pages.CRATES.items():
        if not crate.get("demo"):
            continue
        resp = client.get(f"/x/crate-demo/{key}")
        assert resp.status_code == 200, key
        assert resp.headers["content-type"] == "image/gif", key
        assert len(resp.content) > 0, key


def test_crate_without_demo_returns_404(client):
    """A crate with no demo, and an unknown crate, both 404 (no stray file)."""
    assert client.get("/x/crate-demo/rsx-types").status_code == 404
    assert client.get("/x/crate-demo/does-not-exist").status_code == 404
