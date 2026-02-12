import { test, expect } from "@playwright/test";
import { waitForHTMX, verifyPolling } from "./test_helpers";

test.describe("Topology tab", () => {
  test("loads and shows process graph", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.locator("nav a", { hasText: "Topology" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Process Graph" })).toBeVisible();
    await expect(page.locator("pre")).toContainText("Gateway");
    await expect(page.locator("pre")).toContainText("Risk");
    await expect(page.locator("pre")).toContainText("Marketdata");
  });

  test("has core affinity card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Core Affinity Map" })).toBeVisible();
  });

  test("has CMP connections card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "CMP Connections" })).toBeVisible();
  });

  test("has process list card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Process List" })).toBeVisible();
  });

  // New interactive tests (7 total)

  test("process graph shows nodes for running processes", async ({ page }) => {
    await page.goto("/topology");
    const graph = page.locator("pre");

    // Should show key process names
    await expect(graph).toContainText("Gateway");
    await expect(graph).toContainText("Risk");
    await expect(graph).toContainText("ME");
    await expect(graph).toContainText("Marketdata");
    await expect(graph).toContainText("Recorder");
    await expect(graph).toContainText("Mark");
  });

  test("process graph shows edges for CMP connections", async ({ page }) => {
    await page.goto("/topology");
    const graph = page.locator("pre");

    // Should show connection arrows
    await expect(graph).toContainText("---");
    await expect(graph).toContainText("CMP/UDP");
    await expect(graph).toContainText("WAL/TCP");
  });

  test("core affinity map auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/topology");
    const affinity = page.locator("[hx-get='./x/core-affinity']");

    await verifyPolling(affinity, "every 5s");
  });

  test("core affinity displays process-to-core mapping", async ({ page }) => {
    await page.goto("/topology");
    const affinity = page.locator("[hx-get='./x/core-affinity']");
    await waitForHTMX(page, 2000);

    // Should show core mapping or "no processes"
    const content = await affinity.textContent();
    expect(content).toMatch(/Core|no processes/i);
  });

  test("CMP connections card auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/topology");
    const cmpFlows = page.locator("[hx-get='./x/cmp-flows']");

    await verifyPolling(cmpFlows, "every 2s");
  });

  test("CMP connections show gateway-risk-ME flow", async ({ page }) => {
    await page.goto("/topology");
    const cmpFlows = page.locator("[hx-get='./x/cmp-flows']");
    await waitForHTMX(page, 2000);

    // Should show connection names
    const content = await cmpFlows.textContent();
    expect(content).toMatch(/Gateway.*Risk|Risk.*ME|ME.*Mktdata/i);
  });

  test("process list auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/topology");
    const procList = page.locator("[hx-get='./x/processes']").last();

    await verifyPolling(procList, "every 2s");
  });
});
