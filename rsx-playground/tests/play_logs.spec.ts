import { test, expect } from "@playwright/test";
import { waitForHTMX, waitForRefresh } from "./test_helpers";

test.describe("Logs tab", () => {
  test("loads and has filters", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.locator("nav a", { hasText: "Logs" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Unified Log" })).toBeVisible();
    // Filter inputs are present (hidden inputs + visible search)
    await expect(page.locator("#log-process")).toBeAttached();
    await expect(page.locator("#log-level")).toBeAttached();
    await expect(page.locator("#log-search")).toBeAttached();
    // Visible free-text search
    await expect(page.locator("#log-search-input")).toBeVisible();
    // Pill buttons present
    await expect(page.locator("button#fp-all")).toBeVisible();
    await expect(page.locator("button#fl-all")).toBeVisible();
  });

  test("process pill buttons present", async ({ page }) => {
    await page.goto("/logs");
    for (const id of ["fp-all", "fp-gateway", "fp-risk", "fp-matching",
                       "fp-marketdata", "fp-mark", "fp-recorder"]) {
      await expect(page.locator(`#${id}`)).toBeAttached();
    }
  });

  test("level pill buttons present", async ({ page }) => {
    await page.goto("/logs");
    for (const id of ["fl-all", "fl-error", "fl-warn", "fl-info", "fl-debug"]) {
      await expect(page.locator(`#${id}`)).toBeAttached();
    }
  });

  test("has error aggregation card", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.getByRole("heading", { name: "Error Aggregation" })).toBeVisible();
  });

  test("log table renders with thead and tbody", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");
    await expect(page.locator("#log-table")).toBeVisible();
    await expect(page.locator("#log-table thead")).toBeVisible();
    await expect(page.locator("#log-view")).toBeAttached();
  });

  test("no modal: log-modal is absent from DOM", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.locator("#log-modal")).not.toBeAttached();
  });

  test("quick filters: click gateway pill sets hidden input", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    await page.locator("#fp-gateway").click();
    await waitForHTMX(page);

    const value = await page.evaluate(() => {
      return (document.getElementById("log-process") as HTMLInputElement)?.value;
    });
    expect(value).toBe("gateway");
  });

  test("level pill: click ERR sets log-level hidden input", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    await page.locator("#fl-error").click();
    await waitForHTMX(page);

    const value = await page.evaluate(() => {
      return (document.getElementById("log-level") as HTMLInputElement)?.value;
    });
    expect(value).toBe("error");
  });

  test("free-text search: typing updates log-search after debounce", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    await page.locator("#log-search-input").fill("connected");
    await page.waitForTimeout(400); // debounce

    const value = await page.evaluate(() => {
      return (document.getElementById("log-search") as HTMLInputElement)?.value;
    });
    expect(value).toBe("connected");
  });

  test("clear filters: clears all hidden inputs", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    // Set some filters
    await page.locator("#fp-gateway").click();
    await waitForHTMX(page);
    await page.locator("#fl-error").click();
    await waitForHTMX(page);

    // Click clear filters
    await page.locator("button", { hasText: "clear filters" }).click();
    await waitForHTMX(page);

    const values = await page.evaluate(() => {
      return {
        process: (document.getElementById("log-process") as HTMLInputElement)?.value,
        level: (document.getElementById("log-level") as HTMLInputElement)?.value,
        search: (document.getElementById("log-search") as HTMLInputElement)?.value,
      };
    });
    expect(values.process).toBe("");
    expect(values.level).toBe("");
    expect(values.search).toBe("");
  });

  test("inline accordion: click row expands detail, no modal", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    const rows = page.locator("#log-view tr");
    const count = await rows.count();

    if (count > 0) {
      // Click first data row
      await rows.first().click();

      // Detail div should be visible inside the row
      const detail = rows.first().locator(".log-detail");
      await expect(detail).not.toHaveClass(/hidden/);

      // Summary should be hidden
      const summary = rows.first().locator(".log-summary");
      await expect(summary).toHaveClass(/hidden/);

      // No modal should appear
      await expect(page.locator("#log-modal")).not.toBeAttached();
    }
  });

  test("inline accordion: Escape collapses open row", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    const rows = page.locator("#log-view tr");
    const count = await rows.count();

    if (count > 0) {
      await rows.first().click();
      const detail = rows.first().locator(".log-detail");
      await expect(detail).not.toHaveClass(/hidden/);

      await page.keyboard.press("Escape");
      await expect(detail).toHaveClass(/hidden/);
    }
  });

  test("inline accordion: only one row open at a time", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    const rows = page.locator("#log-view tr");
    const count = await rows.count();

    if (count >= 2) {
      await rows.nth(0).click();
      await rows.nth(1).click();

      // First row's detail should be closed
      const firstDetail = rows.nth(0).locator(".log-detail");
      await expect(firstDetail).toHaveClass(/hidden/);

      // Second row's detail should be open
      const secondDetail = rows.nth(1).locator(".log-detail");
      await expect(secondDetail).not.toHaveClass(/hidden/);
    }
  });

  test("copy button inside expanded row", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    const rows = page.locator("#log-view tr");
    const count = await rows.count();

    if (count > 0) {
      await rows.first().click();
      const copyBtn = rows.first().locator("button", { hasText: "copy" });
      await expect(copyBtn).toBeVisible();
    }
  });

  test("tail toggle button present", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.locator("#tail-toggle")).toBeVisible();
  });

  test("keyboard shortcut / focuses search input", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    await page.locator("h2").first().click();
    await page.keyboard.press("/");

    await expect(page.locator("#log-search-input")).toBeFocused({ timeout: 5000 });
  });

  test("keyboard shortcut Ctrl+L clears all filters", async ({ page }) => {
    await page.goto("/logs");
    await page.locator("#fp-risk").click();
    await waitForHTMX(page);

    await page.keyboard.press("Control+l");
    await waitForHTMX(page);

    const value = await page.evaluate(() => {
      return (document.getElementById("log-process") as HTMLInputElement)?.value;
    });
    expect(value).toBe("");
  });

  test("auto-refresh with filters: process filter persists", async ({ page }) => {
    await page.goto("/logs");

    await page.locator("#fp-risk").click();
    await waitForHTMX(page);

    const initial = await page.evaluate(() => {
      return (document.getElementById("log-process") as HTMLInputElement)?.value;
    });
    expect(initial).toBe("risk");

    await waitForRefresh(2000);

    const after = await page.evaluate(() => {
      return (document.getElementById("log-process") as HTMLInputElement)?.value;
    });
    expect(after).toBe("risk");
  });

  // F7: gateway filter matches gw- log lines
  test("filter_label_to_log_prefix (F7): gateway filter matches gw- lines",
    async ({ request }) => {
      const proc = await request.get("/api/processes");
      const procs = await proc.json() as Array<{
        name: string; state: string;
      }>;
      const gwUp = procs.some(
        (p) => /^gw-/.test(p.name) && p.state === "running",
      );
      const res = await request.get("/x/logs?log-process=gateway");
      expect(res.ok()).toBe(true);
      const html = await res.text();
      if (gwUp) {
        expect(html).toMatch(/\[gw-\d+\]/);
      }
    },
  );

  test("log viewer loads without console errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error"
        && !msg.text().includes("ERR_")
        && !msg.text().includes("htmx")
        && !msg.text().includes("Failed to fetch")) {
        errors.push(msg.text());
      }
    });

    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    await page.locator("#fp-gateway").click();
    await waitForHTMX(page);

    await page.waitForTimeout(1000);
    expect(errors.length).toBe(0);
  });
});
