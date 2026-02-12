import { test, expect } from "@playwright/test";

const TABS = [
  { label: "Overview", path: "/overview" },
  { label: "Topology", path: "/topology" },
  { label: "Book", path: "/book" },
  { label: "Risk", path: "/risk" },
  { label: "WAL", path: "/wal" },
  { label: "Logs", path: "/logs" },
  { label: "Control", path: "/control" },
  { label: "Faults", path: "/faults" },
  { label: "Verify", path: "/verify" },
  { label: "Orders", path: "/orders" },
];

test.describe("Navigation", () => {
  test("all 10 tab links are present", async ({ page }) => {
    await page.goto("/");
    for (const tab of TABS) {
      await expect(
        page.locator("nav a", { hasText: tab.label })
      ).toBeVisible();
    }
  });

  for (const tab of TABS) {
    test(`clicking ${tab.label} navigates`, async ({ page }) => {
      await page.goto("/");
      await page
        .locator("nav a", { hasText: tab.label })
        .click();
      await expect(page).toHaveURL(new RegExp(tab.path));
      await expect(
        page.locator("nav a", { hasText: tab.label })
      ).toHaveClass(/bg-slate-700/);
    });
  }

  test("root shows overview as active", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveTitle(/RSX/);
    await expect(
      page.locator("nav a", { hasText: "Overview" })
    ).toHaveClass(/bg-slate-700/);
  });
});
