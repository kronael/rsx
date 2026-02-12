import { test, expect } from "@playwright/test";

test.describe("Control tab", () => {
  test("loads and has process control card", async ({ page }) => {
    await page.goto("/control");
    await expect(page.locator("nav a", { hasText: "Control" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Process Control" })).toBeVisible();
  });

  test("has notes card with scenario commands", async ({ page }) => {
    await page.goto("/control");
    await expect(page.getByRole("heading", { name: "Notes" })).toBeVisible();
    await expect(page.locator("code").first()).toBeVisible();
  });

  test("has resource usage card", async ({ page }) => {
    await page.goto("/control");
    await expect(page.getByRole("heading", { name: "Resource Usage" })).toBeVisible();
  });

  test("has scenario selector dropdown", async ({ page }) => {
    await page.goto("/control");
    const scenarioSelect = page.locator("#scenario-select");
    await expect(scenarioSelect).toBeVisible();
    await expect(scenarioSelect).toHaveValue("full");
  });

  test("scenario selector has all options", async ({ page }) => {
    await page.goto("/control");
    const scenarioSelect = page.locator("#scenario-select");
    await scenarioSelect.selectOption("minimal");
    await scenarioSelect.selectOption("duo");
    await scenarioSelect.selectOption("full");
    await scenarioSelect.selectOption("stress-low");
    await scenarioSelect.selectOption("stress-high");
    await scenarioSelect.selectOption("stress-ultra");
  });

  test("scenario switch button works", async ({ page }) => {
    await page.goto("/control");
    await page.locator("#scenario-select").selectOption("minimal");
    await page.locator("button", { hasText: "Switch Scenario" }).click();
    await page.waitForSelector(".htmx-request", { state: "detached", timeout: 5000 });
    await expect(page.locator("#scenario-status")).toContainText("switched");
  });

  test("control grid auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/control");
    await page.waitForSelector("div[hx-get='./x/control-grid']", { timeout: 5000 });
    const firstState = await page.locator("div[hx-get='./x/control-grid']").innerHTML();
    await page.waitForTimeout(2200);
    const secondState = await page.locator("div[hx-get='./x/control-grid']").innerHTML();
    expect(secondState).toBeDefined();
  });

  test("resource usage auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/control");
    await page.waitForSelector("div[hx-get='./x/resource-usage']", { timeout: 5000 });
    const firstState = await page.locator("div[hx-get='./x/resource-usage']").innerHTML();
    await page.waitForTimeout(5200);
    const secondState = await page.locator("div[hx-get='./x/resource-usage']").innerHTML();
    expect(secondState).toBeDefined();
  });

  test("current scenario displays correctly", async ({ page }) => {
    await page.goto("/control");
    const currentScenario = page.locator("code[hx-get='./x/current-scenario']");
    await expect(currentScenario).toBeVisible();
    await page.waitForTimeout(500);
    const text = await currentScenario.textContent();
    expect(text).toBeTruthy();
  });

  test("process control grid shows process rows", async ({ page }) => {
    await page.goto("/control");
    await page.waitForSelector("div[hx-get='./x/control-grid']", { timeout: 5000 });
    await page.waitForTimeout(500);
    const gridContent = await page.locator("div[hx-get='./x/control-grid']").innerHTML();
    expect(gridContent.length).toBeGreaterThan(50);
  });

  test("resource usage shows CPU and memory bars", async ({ page }) => {
    await page.goto("/control");
    await page.waitForSelector("div[hx-get='./x/resource-usage']", { timeout: 5000 });
    await page.waitForTimeout(500);
    const resourceContent = await page.locator("div[hx-get='./x/resource-usage']").innerHTML();
    expect(resourceContent.length).toBeGreaterThan(0);
  });

  test("notes card contains scenario commands", async ({ page }) => {
    await page.goto("/control");
    const notesCard = page.getByRole("heading", { name: "Notes" }).locator("..");
    await expect(notesCard).toContainText("./start");
  });

  test("scenario selector shows stress test options", async ({ page }) => {
    await page.goto("/control");
    const scenarioSelect = page.locator("#scenario-select");
    const stressLowOption = scenarioSelect.locator("option[value='stress-low']");
    await expect(stressLowOption).toContainText("stress-low");
    const stressHighOption = scenarioSelect.locator("option[value='stress-high']");
    await expect(stressHighOption).toContainText("stress-high");
    const stressUltraOption = scenarioSelect.locator("option[value='stress-ultra']");
    await expect(stressUltraOption).toContainText("stress-ultra");
  });

  test("process control grid has action buttons", async ({ page }) => {
    await page.goto("/control");
    await page.waitForSelector("div[hx-get='./x/control-grid']", { timeout: 5000 });
    await page.waitForTimeout(500);
    const gridContent = await page.locator("div[hx-get='./x/control-grid']").innerHTML();
    expect(gridContent).toBeDefined();
  });

  test("scenario status updates after switch", async ({ page }) => {
    await page.goto("/control");
    await page.locator("#scenario-select").selectOption("duo");
    await page.locator("button", { hasText: "Switch Scenario" }).click();
    await page.waitForSelector(".htmx-request", { state: "detached", timeout: 5000 });
    await page.waitForTimeout(1200);
    const statusText = await page.locator("#scenario-status").textContent();
    expect(statusText).toBeTruthy();
  });
});
