/**
 * Infra smoke test: session lifecycle (create / reuse / teardown).
 *
 * global-setup.ts already holds the active session (create).
 * These tests verify:
 *   1. create  — allocate returns a valid session_id + run_id
 *                (indirectly: global-setup succeeded)
 *   2. reuse   — /api/sessions/status reflects the active session
 *                across calls (session persists between requests)
 *   3. teardown — release with wrong id → 400 (guard is live);
 *                 re-allocate while session active → 409 (guard is live)
 *
 * All product shards depend on "infra-smoke" via playwright.config.ts.
 * If any test here fails, product shards are skipped immediately.
 */

import { test, expect } from "@playwright/test";

// ── 1. create ────────────────────────────────────────────────────────
// global-setup allocated the session before this test runs.
// We verify the result is visible via /api/sessions/status.
test("session create: status shows active session after global-setup", async ({
  request,
}) => {
  const res = await request.get("/api/sessions/status");
  expect(res.ok()).toBe(true);
  const body = await res.json();
  expect(body.active).toBe(true);
  expect(typeof (body.session_id ?? body.active_id)).toBe("string");
  expect(typeof body.run_id).toBe("string");
  expect(body.run_id.length).toBeGreaterThan(0);
});

// ── 2. reuse ─────────────────────────────────────────────────────────
// Two back-to-back status calls must return the same active_id.
// This proves the session persists across requests (reuse semantics).
test("session reuse: repeated status calls return same session id", async ({
  request,
}) => {
  const r1 = await request.get("/api/sessions/status");
  const r2 = await request.get("/api/sessions/status");
  expect(r1.ok()).toBe(true);
  expect(r2.ok()).toBe(true);
  const b1 = await r1.json();
  const b2 = await r2.json();
  expect(b1.active).toBe(true);
  expect(b2.active).toBe(true);
  // Same lock token across both calls
  expect(b1.active_id).toBe(b2.active_id);
  // run_id is stable within a session
  expect(b1.run_id).toBe(b2.run_id);
});

// ── 3a. teardown guard: wrong session_id rejected ────────────────────
// Release with a fabricated session_id must return 400, not 200.
// Confirms the teardown guard is live and would not silently accept
// rogue releases.
test(
  "session teardown guard: release with wrong id returns 400",
  async ({ request }) => {
    const res = await request.post("/api/sessions/release", {
      data: { session_id: "00000000000000000000000000000000" },
    });
    expect(res.status()).toBe(400);
    const body = await res.json();
    expect(body.error).toMatch(/mismatch/);
    // Session still active after rejected release
    const status = await request.get("/api/sessions/status");
    const sb = await status.json();
    expect(sb.active).toBe(true);
  },
);

// ── 3b. teardown guard: re-allocate while active returns 409 ─────────
// A second allocate while the global session is held must return 409.
// This is the collision guard that makes teardown detectable: if the
// guard fires, the active session is still alive (not torn down).
test(
  "session teardown guard: re-allocate while active returns 409",
  async ({ request }) => {
    const res = await request.post("/api/sessions/allocate");
    expect(res.status()).toBe(409);
    const body = await res.json();
    expect(body.error).toMatch(/collision/);
    expect(typeof body.active_id).toBe("string");
    expect(body.active_id.length).toBeGreaterThan(0);
  },
);

// ── 3c. teardown: age increments between calls ────────────────────────
// age_s at t2 must be >= age_s at t1, confirming the session is the
// same one (not re-created between calls) and its age is tracked.
test("session reuse: age_s increments monotonically across calls", async ({
  request,
}) => {
  const r1 = await request.get("/api/sessions/status");
  const b1 = await r1.json();
  // Small sleep to ensure measurable delta
  await new Promise((r) => setTimeout(r, 100));
  const r2 = await request.get("/api/sessions/status");
  const b2 = await r2.json();
  expect(b2.age_s).toBeGreaterThanOrEqual(b1.age_s);
  expect(b2.ttl_remaining_s).toBeLessThanOrEqual(b1.ttl_remaining_s + 1);
});
