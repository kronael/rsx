import { test, expect } from "@playwright/test";

// /walkthrough was folded into Overview: the hero, Start-All
// launcher, and architecture/lifecycle diagrams now live on
// /overview, and /walkthrough redirects there (never 404s).

test.describe("Walkthrough (redirect + folded Overview)", () => {
  test("/walkthrough redirects to /overview", async ({ page }) => {
    await page.goto("/walkthrough");
    await expect(page).toHaveURL(/\/overview/);
  });

  test("overview shows the RSX hero title", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.locator("h1")).toContainText("RSX Exchange");
  });

  test("architecture + lifecycle anchors exist", async ({
    page,
  }) => {
    await page.goto("/overview");
    for (const id of ["big-picture", "order-lifecycle"]) {
      await expect(page.locator(`#${id}`)).toBeAttached();
    }
  });

  test("start-all launcher is reachable", async ({ page }) => {
    await page.goto("/overview");
    await expect(
      page.locator("button", { hasText: "Build & Start All" }),
    ).toBeVisible();
  });
});

test.describe("Narrative hints", () => {
  test("overview shows a hint with a next link", async ({
    page,
  }) => {
    await page.goto("/overview");
    const hint = page.locator(".rsx-hint");
    await expect(hint).toBeVisible();
    await expect(hint).toContainText("next");
    await expect(
      hint.getByRole("link", { name: /Topology/ }),
    ).toBeVisible();
  });

  test("ops pages (logs) show no narrative hint", async ({
    page,
  }) => {
    await page.goto("/logs");
    await expect(page.locator(".rsx-hint")).toHaveCount(0);
  });

  test("hide toggle is sticky site-wide", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.locator(".rsx-hint")).toBeVisible();

    await page
      .locator("button", { hasText: "Hide hints" })
      .click();
    await expect(page.locator(".rsx-hint")).toBeHidden();

    // Sticky across navigation via localStorage.rsxHints.
    await page.goto("/topology");
    await expect(page.locator(".rsx-hint")).toBeHidden();
    await expect(
      page.locator("button", { hasText: "Show hints" }),
    ).toBeVisible();

    // Restore.
    await page
      .locator("button", { hasText: "Show hints" })
      .click();
    await expect(page.locator(".rsx-hint")).toBeVisible();
  });
});
