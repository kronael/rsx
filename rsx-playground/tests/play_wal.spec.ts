import { test, expect } from "@playwright/test";

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
});
