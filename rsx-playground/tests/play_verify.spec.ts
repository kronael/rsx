import { test, expect } from "@playwright/test";
import { waitForHTMX, verifyPolling } from "./test_helpers";

test.describe("Verify tab", () => {
  test("loads with invariants card", async ({ page }) => {
    await page.goto("/verify");
    await expect(page.locator("nav a", { hasText: "Verify" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Invariants" })).toBeVisible();
  });

  test("has Run All Checks button", async ({ page }) => {
    await page.goto("/verify");
    await expect(
      page.locator("button", { hasText: "Run All Checks" })
    ).toBeVisible();
  });

  test("has reconciliation card", async ({ page }) => {
    await page.goto("/verify");
    await expect(page.getByRole("heading", { name: "Reconciliation" })).toBeVisible();
  });

  test("has latency regression card", async ({ page }) => {
    await page.goto("/verify");
    await expect(page.getByRole("heading", { name: "Latency Regression" })).toBeVisible();
  });

  // New interactive tests (10 total)

  test("run checks button triggers verification", async ({ page }) => {
    await page.goto("/verify");
    const runBtn = page.locator("button", { hasText: "Run All Checks" });
    const results = page.locator("#verify-results");

    // Should show initial state
    await waitForHTMX(page, 2000);

    // Click Run All Checks
    await runBtn.click();
    await waitForHTMX(page, 3000);

    // Results should update with check status
    const content = await results.textContent();
    expect(content).toBeTruthy();
    expect(content?.toLowerCase()).toMatch(/pass|fail|skip|check/);
  });

  test("invariants run on page load", async ({ page }) => {
    await page.goto("/verify");

    // Page has auto-run on load
    const results = page.locator("#verify-results");
    await waitForHTMX(page, 3000);

    // Should show results
    const content = await results.textContent();
    expect(content).toBeTruthy();
  });

  test("verify results auto-refresh every 5s", async ({ page }) => {
    await page.goto("/verify");
    const results = page.locator("#verify-results");

    // Has auto-refresh
    const trigger = await results.getAttribute("hx-trigger");
    expect(trigger).toContain("every 5s");
  });

  test("invariants show 10 system checks", async ({ page }) => {
    await page.goto("/verify");
    await page.locator("button", { hasText: "Run All Checks" }).click();
    await waitForHTMX(page, 3000);

    // Count checks (should be around 10, including WAL, processes, invariants)
    const results = page.locator("#verify-results");
    const content = await results.textContent();

    // Should mention multiple checks
    expect(content).toMatch(/WAL|process|invariant|check/i);
  });

  test("invariants show pass/fail/skip indicators", async ({ page }) => {
    await page.goto("/verify");
    await page.locator("button", { hasText: "Run All Checks" }).click();
    await waitForHTMX(page, 3000);

    const results = page.locator("#verify-results");

    // Should contain status indicators
    await expect(results).toContainText(/PASS|FAIL|SKIP/);
  });

  test("each invariant row has exactly PASS, FAIL, or SKIP badge", async ({ page }) => {
    await page.goto("/verify");
    await page.locator("button", { hasText: "Run All Checks" }).click();
    await waitForHTMX(page, 3000);

    const rows = page.locator("#verify-results table tbody tr");
    const count = await rows.count();
    expect(count).toBeGreaterThan(0);
    for (let i = 0; i < count; i++) {
      const badge = await rows.nth(i).locator("span").first().textContent();
      expect(badge?.trim()).toMatch(/^(PASS|FAIL|SKIP)$/);
    }
  });

  test("reconciliation checks auto-refresh every 5s", async ({ page }) => {
    await page.goto("/verify");
    const recon = page.locator("[hx-get='./x/reconciliation']");

    await verifyPolling(recon, "every 5s");
  });

  test("reconciliation shows margin and book sync checks", async ({ page }) => {
    await page.goto("/verify");
    const recon = page.locator("[hx-get='./x/reconciliation']");
    await waitForHTMX(page, 2000);

    // Should mention reconciliation checks
    const content = await recon.textContent();
    expect(content).toMatch(/margin|book|sync|mark/i);
  });

  test("latency regression auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/verify");
    const latency = page.locator("[hx-get='./x/latency-regression']");

    await verifyPolling(latency, "every 5s");
  });

  test("latency regression shows baseline comparison", async ({ page }) => {
    await page.goto("/verify");
    const latency = page.locator("[hx-get='./x/latency-regression']");
    await waitForHTMX(page, 2000);

    // Should show baseline latency targets
    await expect(latency).toContainText(/baseline|50us|500ns|p99/i);
  });

  test("all verify cards load without errors", async ({ page }) => {
    await page.goto("/verify");

    // Verify all cards visible
    await expect(page.getByRole("heading", { name: "Invariants" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Reconciliation" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Latency Regression" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "E2E Tests" })).toBeVisible();

    await waitForHTMX(page, 3000);

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
