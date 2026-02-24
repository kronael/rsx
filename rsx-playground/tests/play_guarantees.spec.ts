import { test, expect, type APIRequestContext } from "@playwright/test";
import { waitForHTMX } from "./test_helpers";

type Check = { name: string; status: string; detail: string };

// DB-dependent checks fail in sim mode (fills bypass ME→PG)
const DB_SKIP = ["Position", "Funding"];

async function runChecks(
  request: APIRequestContext,
): Promise<Check[]> {
  const res = await request.post("/api/verify/run-json");
  expect(res.ok()).toBeTruthy();
  return (await res.json()).checks as Check[];
}

function nonDbFails(checks: Check[]): Check[] {
  return checks.filter(
    (c) =>
      c.status === "fail" &&
      !DB_SKIP.some((d) => c.name.includes(d)),
  );
}

function findCheck(checks: Check[], needle: string) {
  const c = checks.find((c) => c.name.includes(needle));
  expect(c, `missing check: ${needle}`).toBeDefined();
  return c!;
}

async function order(
  request: APIRequestContext,
  opts: Record<string, string> = {},
) {
  return request.post("/api/orders/test", {
    headers: {
      "content-type": "application/x-www-form-urlencoded",
      ...(opts.key
        ? { "x-idempotency-key": opts.key }
        : {}),
    },
    data: new URLSearchParams({
      symbol_id: opts.symbol_id ?? "10",
      side: opts.side ?? "buy",
      price: opts.price ?? "55000",
      qty: opts.qty ?? "10.0",
      user_id: opts.user_id ?? "1",
    }).toString(),
  });
}

async function cross(request: APIRequestContext) {
  await order(request, {
    side: "buy", price: "56000", user_id: "1",
  });
  await order(request, {
    side: "sell", price: "54000", user_id: "2",
  });
  await new Promise((r) => setTimeout(r, 500));
}

const sleep = (ms: number) =>
  new Promise((r) => setTimeout(r, ms));

test.describe("Invariants", () => {
  test("no non-DB invariant fails", async ({ request }) => {
    const checks = await runChecks(request);
    const failed = nonDbFails(checks);
    expect(
      failed, JSON.stringify(failed),
    ).toHaveLength(0);
  });

  test("no crossed book after batch", async ({
    request,
  }) => {
    await request.post("/api/orders/batch");
    await sleep(500);
    const c = findCheck(
      await runChecks(request), "crossed book",
    );
    expect(c.status).not.toBe("fail");
  });

  test("fills precede ORDER_DONE", async ({ request }) => {
    await cross(request);
    const c = findCheck(
      await runChecks(request), "Fills precede",
    );
    expect(c.status).not.toBe("fail");
  });

  test("exactly-one completion", async ({ request }) => {
    await request.post("/api/orders/batch");
    await sleep(300);
    const c = findCheck(
      await runChecks(request), "Exactly-one",
    );
    expect(c.status).not.toBe("fail");
  });

  test("tips monotonic", async ({ request }) => {
    const c = findCheck(
      await runChecks(request), "Tips monotonic",
    );
    expect(c.status).not.toBe("fail");
  });
});

test.describe("Fill durability", () => {
  test("WAL timeline has fill events", async ({
    request,
  }) => {
    await cross(request);
    const html = await (
      await request.get("/x/wal-timeline")
    ).text();
    expect(html).toMatch(/fill|FILL|order|RECORD/i);
  });

  test("idempotency dedup rejects duplicate", async ({
    request,
  }) => {
    const key = `dedup-${Date.now()}`;
    await order(request, { key });
    const dup = await order(request, { key });
    expect(await dup.text()).toContain("duplicate");
  });
});

test.describe("Order at-most-once", () => {
  test("order returns ack", async ({ request }) => {
    const html = await (await order(request)).text();
    expect(html).toMatch(
      /accepted|queued|submitted|simulated/i,
    );
  });

  test("concurrent dup burst deduped", async ({
    request,
  }) => {
    const key = `burst-${Date.now()}`;
    const results = await Promise.all(
      [1, 2, 3].map(() => order(request, { key })),
    );
    const texts = await Promise.all(
      results.map((r) => r.text()),
    );
    const dupes = texts.filter(
      (t) => t.includes("duplicate"),
    );
    expect(dupes.length).toBeGreaterThanOrEqual(2);
  });
});

test.describe("Crash recovery", () => {
  test("invariants hold after kill+restart", async ({
    request,
  }) => {
    await request.post("/api/processes/me-pengu/kill");
    await sleep(500);
    await request.post("/api/processes/me-pengu/restart");
    await sleep(1000);
    const failed = nonDbFails(await runChecks(request))
      .filter((c) => !c.name.includes("processes"));
    expect(
      failed, JSON.stringify(failed),
    ).toHaveLength(0);
  });

  test("invariants hold after all-stop → all-start",
    async ({ request }) => {
      const session = await request.post(
        "/api/sessions/allocate",
      );
      const runId = session.ok()
        ? (await session.json()).run_id ?? ""
        : "";
      await request.post("/api/processes/all/stop", {
        headers: {
          "x-run-id": runId, "x-confirm": "yes",
        },
      });
      await sleep(500);
      await request.post("/api/processes/all/start");
      await sleep(2000);
      const failed = nonDbFails(await runChecks(request))
        .filter((c) => !c.name.includes("processes"));
      expect(
        failed, JSON.stringify(failed),
      ).toHaveLength(0);
    },
  );

  test("book page loads after recovery", async ({
    page,
  }) => {
    await page.goto("/book");
    await waitForHTMX(page, 2000);
    await expect(page.locator("body")).toContainText(/\d/);
  });

  test("order works after recovery", async ({
    request,
  }) => {
    const res = await order(request);
    expect(res.status()).toBeLessThan(500);
  });
});

test.describe("Backpressure", () => {
  test("50 concurrent orders → no 500s, < 5s", async ({
    request,
  }) => {
    const start = Date.now();
    const results = await Promise.all(
      Array.from({ length: 50 }, (_, i) =>
        order(request, { price: `${55000 + i}` }),
      ),
    );
    expect(Date.now() - start).toBeLessThan(5000);
    const errors = results.filter(
      (r) => r.status() >= 500,
    );
    expect(errors).toHaveLength(0);
  });
});

test.describe("Reconciliation", () => {
  test("returns valid check results", async ({
    request,
  }) => {
    const res = await request.get("/x/reconciliation");
    expect(res.ok()).toBeTruthy();
    const html = await res.text();
    expect(html).toMatch(/PASS|FAIL|SKIP/i);
    if (html.includes("symbols")) {
      expect(html).not.toMatch(/\d+\/\d+ mismatch/);
    }
  });
});
