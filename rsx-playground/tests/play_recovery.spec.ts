import { test, expect } from "@playwright/test";
import { waitForHTMX, verifyPolling } from "./test_helpers";

// Static tests run without a cluster. The crash→heal test
// gates on a live cluster (like the e2e_* tests): it skips
// when no managed process is running.
test.describe("Recovery explorer", () => {
  test("tab loads and is active", async ({ page }) => {
    await page.goto("/recovery");
    await expect(page.locator("nav a", { hasText: "Recovery" }))
      .toHaveClass(/bg-slate-700/);
    await expect(
      page.getByRole("heading", { name: "Inject fault" })
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Recovery feed (live)" })
    ).toBeVisible();
  });

  test("controls partial renders the crash section", async ({
    request,
  }) => {
    const res = await request.get("/x/recovery-controls");
    expect(res.ok()).toBe(true);
    expect(await res.text()).toContain("process crash");
  });

  test("feed partial renders", async ({ request }) => {
    const res = await request.get("/x/recovery-feed");
    expect(res.ok()).toBe(true);
    expect((await res.text()).length).toBeGreaterThan(0);
  });

  test("feed polls every 1s", async ({ page }) => {
    await page.goto("/recovery");
    const feed = page.locator("[hx-get='./x/recovery-feed']");
    await verifyPolling(feed, "every 1s");
  });

  test("crash a process → restart event → heals green", async ({
    page,
    request,
  }) => {
    const controls = await (
      await request.get("/x/recovery-controls")
    ).text();
    test.skip(
      !controls.includes("Crash"),
      "no running processes (needs a live cluster)"
    );

    await page.goto("/recovery");
    await waitForHTMX(page, 3000);

    page.on("dialog", (d) => d.accept());
    const crashBtns = page.locator("button", {
      hasText: /^Crash$/,
    });
    expect(await crashBtns.count()).toBeGreaterThan(0);
    await crashBtns.first().click();

    // inject result confirms the crash landed
    await expect(page.locator("#recovery-inject-result")).toContainText(
      /crashed/,
      { timeout: 5000 }
    );

    // the watcher restarts within a few seconds; the feed
    // returns a green "healthy" event for the cluster.
    const feed = page.locator("[hx-get='./x/recovery-feed']");
    await expect(feed).toContainText("healthy", { timeout: 30000 });
  });
});
