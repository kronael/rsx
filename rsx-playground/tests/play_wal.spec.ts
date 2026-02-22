import { test, expect } from "@playwright/test";
import { waitForHTMX, waitForRefresh, verifyPolling } from "./test_helpers";

test.describe("WAL tab", () => {
  test("loads with per-process WAL state card", async ({ page }) => {
    await page.goto("/wal");
    await expect(page.locator("nav a", { hasText: "WAL" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Per-Process WAL State" })).toBeVisible();
  });

  test("has lag dashboard card", async ({ page }) => {
    await page.goto("/wal");
    await expect(page.getByRole("heading", { name: "Lag Dashboard" })).toBeVisible();
  });

  test("has WAL files card", async ({ page }) => {
    await page.goto("/wal");
    await expect(page.getByRole("heading", { name: "WAL Files" })).toBeVisible();
  });

  test("has timeline card with filter", async ({ page }) => {
    await page.goto("/wal");
    await expect(page.getByRole("heading", { name: "Timeline" })).toBeVisible();
    await expect(page.locator("#wal-filter")).toBeVisible();
  });

  // New interactive tests (12 total)

  test("per-process WAL state auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/wal");
    const walDetail = page.locator("[hx-get='./x/wal-detail']");

    await verifyPolling(walDetail, "every 2s");
  });

  test("per-process WAL state shows streams", async ({ page }) => {
    await page.goto("/wal");
    const walDetail = page.locator("[hx-get='./x/wal-detail']");
    await waitForHTMX(page, 2000);

    // Should show stream info or "no WAL streams" message
    const content = await walDetail.textContent();
    expect(content).toBeTruthy();
  });

  test("lag dashboard auto-refreshes every 1s", async ({ page }) => {
    await page.goto("/wal");
    const lagDash = page.locator("[hx-get='./x/wal-lag']");

    await verifyPolling(lagDash, "every 1s");
  });

  test("lag dashboard shows producer-consumer gap placeholder", async ({ page }) => {
    await page.goto("/wal");
    const lagDash = page.locator("[hx-get='./x/wal-lag']");
    await waitForHTMX(page, 2000);

    // Should show lag data or stream table
    await expect(lagDash).toContainText(/start RSX|lag|no data|Stream|active|idle/i);
  });

  test("timeline filter has event type options", async ({ page }) => {
    await page.goto("/wal");
    const filter = page.locator("#wal-filter");

    // Verify options exist (options are not "visible" in browsers, just present in DOM)
    await expect(filter.locator("option[value='']")).toHaveText("all");
    await expect(filter.locator("option[value='ORDER_ACCEPTED']")).toHaveCount(1);
    await expect(filter.locator("option[value='FILL']")).toHaveCount(1);
    await expect(filter.locator("option[value='MARGIN_CHECK']")).toHaveCount(1);
  });

  test("timeline auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/wal");
    const timeline = page.locator("[hx-get='./x/wal-timeline']");

    await verifyPolling(timeline, "every 2s");
  });

  test("timeline shows placeholder when no data", async ({ page }) => {
    await page.goto("/wal");
    const timeline = page.locator("[hx-get='./x/wal-timeline']");
    await waitForHTMX(page, 2000);

    // Should show placeholder or event data
    await expect(timeline).toContainText(/no WAL events|no data|timeline|Seq/i);
  });

  test("WAL files card auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/wal");
    const files = page.locator("[hx-get='./x/wal-files']");

    await verifyPolling(files, "every 5s");
  });

  test("WAL files card has verify and dump buttons", async ({ page }) => {
    await page.goto("/wal");

    const verifyBtn = page.locator("button", { hasText: "Verify" });
    await expect(verifyBtn).toBeVisible();
    await expect(verifyBtn).toHaveAttribute("hx-post", "./api/wal/verify");

    const dumpBtn = page.locator("button", { hasText: "Dump JSON" });
    await expect(dumpBtn).toBeVisible();
    await expect(dumpBtn).toHaveAttribute("hx-post", "./api/wal/dump");
  });

  test("verify button triggers WAL integrity check", async ({ page }) => {
    await page.goto("/wal");
    const verifyBtn = page.locator("button", { hasText: "Verify" });

    await verifyBtn.click();
    await waitForHTMX(page);

    // Should complete (no error thrown)
    await expect(verifyBtn).toBeVisible();
  });

  test("dump JSON button triggers WAL dump action", async ({ page }) => {
    await page.goto("/wal");
    const dumpBtn = page.locator("button", { hasText: "Dump JSON" });

    await dumpBtn.click();
    await waitForHTMX(page);

    await expect(dumpBtn).toBeVisible();
  });

  test("timeline shows events after order submission", async ({
    page,
    request,
  }) => {
    // Submit an aggressive buy that crosses the maker's ask
    await request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "buy",
        order_type: "limit",
        price: "51000",
        qty: "1",
        user_id: "1",
      },
    });

    await page.goto("/wal");
    const timeline = page.locator("[hx-get='./x/wal-timeline']");

    // Wait up to 8s for WAL records to propagate and HTMX to refresh
    await expect(timeline).not.toContainText(
      /no WAL events recorded/i,
      { timeout: 8000 },
    );

    // Should show a table row with a known event type
    await expect(timeline).toContainText(/BBO|FILL/i, { timeout: 8000 });
  });

  test("all WAL cards load without errors", async ({ page }) => {
    await page.goto("/wal");

    // Verify all cards visible
    await expect(page.getByRole("heading", { name: "Per-Process WAL State" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Lag Dashboard" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Rotation / Tip Health" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Timeline" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "WAL Files" })).toBeVisible();

    await waitForHTMX(page, 2000);

    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("ERR_")) {
        errors.push(msg.text());
      }
    });

    await page.waitForTimeout(1000);
    expect(errors.length).toBe(0);
  });
});
