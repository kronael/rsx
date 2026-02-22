/**
 * Orchestration smoke test: session collision lifecycle.
 *
 * Reproduces and prevents "Session ID already in use" collisions by
 * exercising the full release → re-allocate → collision cycle:
 *
 *   1. release   — release the active session (proves teardown works)
 *   2. re-alloc  — new caller allocates immediately (no stale lock)
 *   3. collision — third caller gets 409 while new session is live
 *   4. restore   — release new session; global-setup re-allocates so
 *                  downstream tests see a clean active session
 *
 * This spec runs as the "orch-smoke" project which depends on
 * "session-preflight" (collision guard is verified first).
 *
 * WARNING: this spec mutates shared session state.  It must run
 * workers:1 and must be the sole occupant of the orch-smoke shard.
 * It restores the session at the end of the final test via afterAll.
 */

import { test, expect, request as newRequest } from "@playwright/test";

const BASE = "http://localhost:49171";

// Captured by test 1 (release) and restored by afterAll.
let restoredSessionId: string | null = null;
let restoredRunId: string | null = null;

// ── helpers ─────────────────────────────────────────────────────────

async function statusBody(req: Parameters<typeof fetch>[0]): Promise<{
  active: boolean;
  active_id?: string;
  run_id?: string;
  age_s?: number;
  ttl_remaining_s?: number;
  stale?: boolean;
}> {
  const r = await fetch(`${BASE}/api/sessions/status`);
  return r.json();
}

// ── afterAll: restore session so infra-smoke tests still pass ───────

test.afterAll(async () => {
  // If the restore step ran successfully, re-allocate a fresh session
  // so that teardown in global-setup can still release it cleanly.
  // This prevents a cascade failure in shards that run after orch-smoke.
  if (restoredSessionId !== null) return; // already restored in test 4

  // Fallback: allocate a throwaway session if something went wrong.
  await fetch(`${BASE}/api/sessions/allocate`, { method: "POST" });
});

// ── test 1: release ──────────────────────────────────────────────────

test(
  "orch collision: release active session succeeds",
  async ({ request }) => {
    // Get the current active session id first.
    const statusRes = await request.get("/api/sessions/status");
    expect(statusRes.ok()).toBe(true);
    const status = await statusRes.json();
    expect(status.active).toBe(true);
    const activeId: string = status.active_id;
    expect(activeId.length).toBeGreaterThan(0);

    // Release it.
    const releaseRes = await request.post("/api/sessions/release", {
      data: { session_id: activeId },
    });
    expect(releaseRes.ok()).toBe(true);
    const releaseBody = await releaseRes.json();
    expect(releaseBody.ok).toBe(true);

    // Confirm session is gone.
    const afterRes = await request.get("/api/sessions/status");
    const after = await afterRes.json();
    expect(after.active).toBe(false);
  },
);

// ── test 2: re-allocate after release ───────────────────────────────

test(
  "orch collision: re-allocate succeeds immediately after release",
  async ({ request }) => {
    // After test 1 released the session, allocate a new one.
    const allocRes = await request.post("/api/sessions/allocate");
    expect(allocRes.ok()).toBe(true);
    const body = await allocRes.json();
    expect(body.ok).toBe(true);
    expect(typeof body.session_id).toBe("string");
    expect(body.session_id.length).toBeGreaterThan(0);
    expect(typeof body.run_id).toBe("string");
    expect(body.run_id.length).toBeGreaterThan(0);
    // session_id and run_id are distinct tokens.
    expect(body.session_id).not.toBe(body.run_id);

    // Capture for test 3 and afterAll restore.
    restoredSessionId = body.session_id;
    restoredRunId = body.run_id;

    // Confirm status reflects the new session.
    const statusRes = await request.get("/api/sessions/status");
    const status = await statusRes.json();
    expect(status.active).toBe(true);
    expect(status.active_id).toBe(restoredSessionId);
    expect(status.run_id).toBe(restoredRunId);
  },
);

// ── test 3: concurrent caller blocked while new session is live ──────

test(
  "orch collision: concurrent allocate returns 409 with collision payload",
  async ({ request }) => {
    // A second allocate while test 2's session is live must return 409.
    const res = await request.post("/api/sessions/allocate");
    expect(res.status()).toBe(409);
    const body = await res.json();

    // Error message must mention collision.
    expect(body.error).toMatch(/collision/);
    // active_id echoes the session from test 2.
    expect(body.active_id).toBe(restoredSessionId);
    // age is non-negative and small (< 60s, we just created it).
    expect(typeof body.age_s).toBe("number");
    expect(body.age_s).toBeGreaterThanOrEqual(0);
    expect(body.age_s).toBeLessThan(60);
  },
);

// ── test 4: lease renew extends TTL ─────────────────────────────────

test(
  "orch lease: renew extends TTL and returns ok",
  async ({ request }) => {
    // test 2 allocated restoredSessionId; it must still be active.
    expect(restoredSessionId).not.toBeNull();

    const renewRes = await request.post("/api/sessions/renew", {
      data: { session_id: restoredSessionId },
    });
    expect(renewRes.ok()).toBe(true);
    const body = await renewRes.json();
    expect(body.ok).toBe(true);
    expect(body.session_id).toBe(restoredSessionId);
    // TTL was reset to full SESSION_TTL (1800s); must be > 1700.
    expect(typeof body.ttl_remaining_s).toBe("number");
    expect(body.ttl_remaining_s).toBeGreaterThan(1700);

    // Status must include lease_remaining_s after renew.
    const statusRes = await request.get("/api/sessions/status");
    const status = await statusRes.json();
    expect(typeof status.lease_remaining_s).toBe("number");
    // Renewed session is not stale (lease just reset via renew).
    expect(status.stale).toBe(false);
  },
);

// ── test 4b: idempotent reclaim ──────────────────────────────────────
//
// If the active session owner re-submits allocate with their own
// session_id, they get the same session back (not 409).  This is the
// safe-retry path after a transient failure before start completes.

test(
  "orch lease: idempotent reclaim returns same session",
  async ({ request }) => {
    expect(restoredSessionId).not.toBeNull();

    // Re-allocate presenting the active session_id — must succeed.
    const res = await request.post("/api/sessions/allocate", {
      data: { session_id: restoredSessionId },
    });
    expect(res.ok()).toBe(true);
    const body = await res.json();
    expect(body.ok).toBe(true);
    // session_id and run_id must be unchanged.
    expect(body.session_id).toBe(restoredSessionId);
    expect(typeof body.run_id).toBe("string");
    expect(body.reclaimed).toBe(true);

    // Update restoredRunId in case it matters downstream.
    restoredRunId = body.run_id ?? restoredRunId;
  },
);

// ── test 5: unknown caller blocked while session is live ─────────────
//
// A caller without the current session_id still gets 409.

test(
  "orch collision: unknown caller rejected while session active",
  async ({ request }) => {
    // restoredSessionId is active from test 2 (renewed in test 4).
    expect(restoredSessionId).not.toBeNull();

    // A blank allocate (no session_id in body) must return 409.
    const res = await request.post("/api/sessions/allocate");
    expect(res.status()).toBe(409);

    const body = await res.json();
    expect(body.error).toMatch(/collision/);
    // active_id in the response must match the existing session.
    expect(body.active_id).toBe(restoredSessionId);
    expect(typeof body.age_s).toBe("number");
    expect(body.age_s).toBeGreaterThanOrEqual(0);
    expect(body.age_s).toBeLessThan(60);
  },
);

// ── test 6: renew with wrong session_id rejected ─────────────────────

test(
  "orch lease: renew with wrong session_id returns 409",
  async ({ request }) => {
    const res = await request.post("/api/sessions/renew", {
      data: { session_id: "000000000000000000000000deadbeef" },
    });
    expect(res.status()).toBe(409);
    const body = await res.json();
    expect(body.error).toMatch(/mismatch/);
    expect(body.active_id).toBe(restoredSessionId);
  },
);

// ── test 7: release new session → status is inactive ────────────────

test(
  "orch collision: release new session clears active flag",
  async ({ request }) => {
    expect(restoredSessionId).not.toBeNull();

    const releaseRes = await request.post("/api/sessions/release", {
      data: { session_id: restoredSessionId },
    });
    expect(releaseRes.ok()).toBe(true);

    const statusRes = await request.get("/api/sessions/status");
    const status = await statusRes.json();
    expect(status.active).toBe(false);

    // Signal afterAll that restore was handled here.
    restoredSessionId = null;

    // Re-allocate so downstream tests / global teardown still have a
    // valid session to release.
    const realloc = await request.post("/api/sessions/allocate");
    expect(realloc.ok()).toBe(true);
    const rb = await realloc.json();
    restoredSessionId = rb.session_id ?? null;
  },
);

// ── test 8: concurrent allocate storm ────────────────────────────────
//
// Reproduces the prior "Session ID already in use" failure mode:
// before asyncio.Lock was added, concurrent callers racing past the
// `_active_session is not None` check could each allocate their own
// session, causing downstream processes to collide on the shared lock.
//
// Procedure:
//   1. Release the current session so the slot is open.
//   2. Fire CONCURRENCY simultaneous POST /api/sessions/allocate.
//   3. Exactly one must return 200; the rest must return 409.
//   4. Restore the session for downstream teardown.
//
// If the server-side lock is missing or broken, multiple callers will
// race through and multiple 200s will appear — this test fails CI.

test(
  "orch collision: concurrent allocate storm — exactly one 200, rest 409",
  async () => {
    const CONCURRENCY = 6;

    // Release the restored session first.
    if (restoredSessionId) {
      await fetch(`${BASE}/api/sessions/release`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: restoredSessionId }),
        signal: AbortSignal.timeout(3000),
      });
      restoredSessionId = null;
    }

    // Confirm no active session before the storm.
    const before = await (
      await fetch(`${BASE}/api/sessions/status`)
    ).json();
    expect(before.active).toBe(false);

    // Fire CONCURRENCY allocates simultaneously.
    const results = await Promise.all(
      Array.from({ length: CONCURRENCY }, () =>
        fetch(`${BASE}/api/sessions/allocate`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
          signal: AbortSignal.timeout(5000),
        }).then(async (r) => ({ status: r.status, body: await r.json() }))
      ),
    );

    const successes = results.filter((r) => r.status === 200);
    const collisions = results.filter((r) => r.status === 409);

    // Exactly one caller wins the race.
    expect(successes.length).toBe(1);
    // All others must be rejected with collision.
    expect(collisions.length).toBe(CONCURRENCY - 1);

    // Collision responses must carry the expected shape.
    for (const c of collisions) {
      expect(c.body.error).toMatch(/collision/);
      expect(typeof c.body.active_id).toBe("string");
      expect(c.body.active_id.length).toBeGreaterThan(0);
    }

    // The winning session_id must match the active session.
    const winner = successes[0].body;
    expect(typeof winner.session_id).toBe("string");
    expect(winner.session_id.length).toBeGreaterThan(0);

    const after = await (
      await fetch(`${BASE}/api/sessions/status`)
    ).json();
    expect(after.active).toBe(true);
    expect(after.active_id).toBe(winner.session_id);

    // Collision responses must echo the winner's session_id.
    for (const c of collisions) {
      expect(c.body.active_id).toBe(winner.session_id);
    }

    // Restore for afterAll / global teardown.
    restoredSessionId = winner.session_id;
  },
);
