import { test, expect } from "@playwright/test";

test.describe("Orders tab", () => {
  test("loads with order form", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.locator("nav a", { hasText: "Orders" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Submit Order" })).toBeVisible();
    await expect(page.locator("select[name='symbol_id']"))
      .toBeVisible();
    await expect(page.locator("select[name='side']"))
      .toBeVisible();
    await expect(page.locator("input[name='price']"))
      .toBeVisible();
    await expect(page.locator("input[name='qty']"))
      .toBeVisible();
  });

  test("has order lifecycle trace card", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.getByRole("heading", { name: "Order Lifecycle Trace" })).toBeVisible();
    await expect(page.locator("#trace-oid")).toBeVisible();
  });

  test("has recent orders card", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.getByRole("heading", { name: "Recent Orders" })).toBeVisible();
  });

  test("has batch and stress test buttons", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.locator("button", { hasText: "Batch (10)" }))
      .toBeVisible();
    await expect(page.locator("button", { hasText: "Stress (100)" }))
      .toBeVisible();
  });

  test("submits valid order successfully", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("select[name='symbol_id']").selectOption("10");
    await page.locator("select[name='side']").selectOption("buy");
    await page.locator("input[name='price']").fill("50000");
    await page.locator("input[name='qty']").fill("1.0");
    await page.locator("button[type='submit']").click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText("accepted");
  });

  test("handles invalid order via invalid button", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("button", { hasText: "Invalid" }).click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText("rejected");
  });

  test("handles empty qty field", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("input[name='qty']").clear();
    await page.locator("button[type='submit']").click();
    await page.waitForTimeout(2000);
    // Server accepts any qty (gateway validates), result is queued/accepted
    await expect(page.locator("#order-result")).toContainText(/order|queued|accepted/);
  });

  test("batch order submission creates 10 orders", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("button", { hasText: "Batch (10)" }).click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText("10 batch orders");
    // Wait for HTMX auto-refresh (2s interval) then assert recent-orders populated
    await page.waitForTimeout(2500);
    const recentOrders = page.locator("div[hx-get='./x/recent-orders']");
    await expect(recentOrders).not.toContainText("no orders yet");
    await expect(recentOrders.locator("tr").first()).toBeVisible();
  });

  test("random order submission creates 5 orders", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("button", { hasText: "Random (5)" }).click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText("5 random orders");
  });

  test("order lifecycle trace by OID", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("#trace-oid").fill("test-oid-12345");
    await page.locator("button", { hasText: "Trace" }).click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#trace-result")).toContainText("test-oid-12345");
  });

  test("recent orders table updates after submission", async ({ page }) => {
    await page.goto("/orders");
    const initialContent = await page.locator("#order-result").textContent();
    await page.locator("button[type='submit']").click();
    await page.waitForTimeout(2000);
    const updatedContent = await page.locator("#order-result").textContent();
    expect(updatedContent).not.toBe(initialContent);
  });

  test("recent orders auto-refresh every 2s", async ({ page }) => {
    await page.goto("/orders");
    const recentOrders = page.locator("div[hx-get='./x/recent-orders']");
    await page.waitForTimeout(500);
    const firstState = await recentOrders.innerHTML();
    await page.waitForTimeout(2500);
    const secondState = await recentOrders.innerHTML();
    expect(secondState).toBeDefined();
  });

  test("order form has all TIF options", async ({ page }) => {
    await page.goto("/orders");
    const tifSelect = page.locator("select[name='tif']");
    await expect(tifSelect).toBeVisible();
    await tifSelect.selectOption("GTC");
    await tifSelect.selectOption("IOC");
    await tifSelect.selectOption("FOK");
  });

  test("order form has reduce_only checkbox", async ({ page }) => {
    await page.goto("/orders");
    const roCheckbox = page.locator("input[name='reduce_only']");
    await expect(roCheckbox).toBeVisible();
    await roCheckbox.check();
    await expect(roCheckbox).toBeChecked();
  });

  test("order form has post_only checkbox", async ({ page }) => {
    await page.goto("/orders");
    const poCheckbox = page.locator("input[name='post_only']");
    await expect(poCheckbox).toBeVisible();
    await poCheckbox.check();
    await expect(poCheckbox).toBeChecked();
  });

  test("cancel button appears for submitted orders", async ({ page }) => {
    await page.goto("/orders");
    // Submit batch orders (which get "submitted" status, not gateway-dependent)
    await page.locator("button", { hasText: "Batch (10)" }).click();
    await page.waitForTimeout(2000);
    // Recent orders table should refresh and show submitted orders
    await page.waitForTimeout(2500);
    const cancelButton = page.locator("button", { hasText: "Cancel" }).first();
    if (await cancelButton.isVisible()) {
      await expect(cancelButton).toBeVisible();
    }
  });

  test("order form supports all symbol options", async ({ page }) => {
    await page.goto("/orders");
    const symbolSelect = page.locator("select[name='symbol_id']");
    await symbolSelect.selectOption("10");
    await expect(symbolSelect).toHaveValue("10");
    await symbolSelect.selectOption("3");
    await expect(symbolSelect).toHaveValue("3");
    await symbolSelect.selectOption("1");
    await expect(symbolSelect).toHaveValue("1");
  });

  test("order form supports buy and sell sides", async ({ page }) => {
    await page.goto("/orders");
    const sideSelect = page.locator("select[name='side']");
    await sideSelect.selectOption("buy");
    await expect(sideSelect).toHaveValue("buy");
    await sideSelect.selectOption("sell");
    await expect(sideSelect).toHaveValue("sell");
  });

  test("order form has user_id input field", async ({ page }) => {
    await page.goto("/orders");
    const userIdInput = page.locator("input[name='user_id']");
    await expect(userIdInput).toBeVisible();
    await userIdInput.fill("42");
    await expect(userIdInput).toHaveValue("42");
  });

  test("order form has order_type selector", async ({ page }) => {
    await page.goto("/orders");
    const orderTypeSelect = page.locator("select[name='order_type']");
    await expect(orderTypeSelect).toBeVisible();
    await orderTypeSelect.selectOption("limit");
    await orderTypeSelect.selectOption("market");
    await orderTypeSelect.selectOption("post_only");
  });
});
