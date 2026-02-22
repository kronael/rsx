/**
 * Session preflight — startup collision self-test.
 *
 * global-setup.ts allocates an exclusive session before any tests run.
 * This spec verifies that the guard actually fires: a second POST to
 * /api/sessions/allocate while global-setup holds the lock must return
 * 409 Conflict.  If it doesn't, the orchestrator collision guard is
 * broken and all product tests should be skipped via the Playwright
 * `dependencies` mechanism.
 *
 * Run order: this project is listed first and all other projects declare
 * `dependencies: ["session-preflight"]` so Playwright skips them when
 * this test fails.
 */

import { test, expect } from "@playwright/test";

test(
  "session collision: second allocate returns 409 while run is active",
  async ({ request }) => {
    // global-setup holds the active session; a second allocate must be
    // rejected immediately with 409.
    const res = await request.post("/api/sessions/allocate");

    expect(res.status()).toBe(409);

    const body = await res.json();
    // error message must mention collision
    expect(body.error).toMatch(/collision/);
    // active_id is the session held by global-setup
    expect(typeof body.active_id).toBe("string");
    expect(body.active_id.length).toBeGreaterThan(0);
    // age_s is non-negative
    expect(typeof body.age_s).toBe("number");
    expect(body.age_s).toBeGreaterThanOrEqual(0);
  },
);

test(
  "session status: active session includes run_id and ttl fields",
  async ({ request }) => {
    const res = await request.get("/api/sessions/status");
    expect(res.ok()).toBe(true);
    const body = await res.json();
    expect(body.active).toBe(true);
    // run_id is a distinct per-run identifier (non-empty hex string)
    expect(typeof body.run_id).toBe("string");
    expect(body.run_id.length).toBeGreaterThan(0);
    // active_id is the session lock token
    expect(typeof body.active_id).toBe("string");
    expect(body.active_id.length).toBeGreaterThan(0);
    // run_id and active_id are different UUIDs
    expect(body.run_id).not.toBe(body.active_id);
    // ttl and age fields present
    expect(typeof body.age_s).toBe("number");
    expect(body.age_s).toBeGreaterThanOrEqual(0);
    expect(typeof body.ttl_remaining_s).toBe("number");
    expect(body.ttl_remaining_s).toBeGreaterThan(0);
    expect(body.stale).toBe(false);
  },
);

test(
  "run_id guard: stale run_id rejected with 409 before task dispatch",
  async ({ request }) => {
    // A request to /api/processes/all/start with a fabricated run_id
    // that doesn't match the active session must be rejected 409
    // before any processes are started.
    const res = await request.post(
      "/api/processes/all/start?scenario=minimal&confirm=yes",
      { headers: { "X-Run-Id": "000000000000000000000000deadbeef" } },
    );
    expect(res.status()).toBe(409);
    const body = await res.json();
    expect(body.error).toMatch(/run_id/);
  },
);
