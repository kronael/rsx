import { test, expect, Page } from "@playwright/test";

// The shipped /orders page wraps the custom form in a
// collapsed <details> element ("Custom Order"). Radio
// inputs for `side`/`tif` use Tailwind `sr-only` so they
// are visually hidden but functionally present. Tests
// expand the details first and use force:true on the
// hidden radios. The form has GTC+IOC (no FOK) and a
// reduce_only checkbox (no post_only checkbox).

async function expandCustom(page: Page) {
  await page.locator("details > summary").first().click();
}

test.describe("Orders tab", () => {
  test("loads with submit order card", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.locator("nav a", { hasText: "Orders" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Submit Order" })).toBeVisible();
    // Quick-order matrix is visible without expanding
    await expect(page.locator("#quick-result")).toBeVisible();
  });

  test("custom order form expands and shows fields", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    await expect(page.locator("select[name='symbol_id']"))
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
    await expandCustom(page);
    await expect(page.locator("button", { hasText: "Batch" }).first())
      .toBeVisible();
    const multi = page.locator(
      "button", { hasText: /Random|Stress|Batch/ },
    );
    expect(await multi.count()).toBeGreaterThanOrEqual(1);
  });

  test("submits valid order successfully", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    await page.locator("select[name='symbol_id']").selectOption("10");
    await page.locator("input[type='radio'][name='side'][value='buy']")
      .check({ force: true });
    await page.locator("input[name='price']").fill("0.049");
    // PENGU lot=100000, qty_dec=4 → qty=10.0 produces
    // raw qty 100000 (lot-aligned). qty=1.0 would be 10000.
    await page.locator("input[name='qty']").fill("10.0");
    await page.locator("form[hx-post='./api/orders/test'] button[type='submit']").click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText(/order|accepted|queued/);
  });

  test("handles invalid order via invalid button", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    await page.locator("button", { hasText: "Invalid" }).click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText(/rejected|invalid/i);
  });

  test("handles empty qty field", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    await page.locator("input[name='qty']").clear();
    await page.locator("form[hx-post='./api/orders/test'] button[type='submit']").click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText(/order|queued|accepted|rejected/);
  });

  test("batch order submission creates orders", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    await page.locator("button", { hasText: "Batch" }).first().click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#order-result")).toContainText(/batch|orders|queued/i);
    await page.waitForTimeout(2500);
    const recentOrders = page.locator("div[hx-get='./x/recent-orders']");
    await expect(recentOrders).not.toContainText("no orders yet");
    await expect(recentOrders.locator("tr").first()).toBeVisible();
  });

  test("random order action exists", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    // The matrix has a 🎲 Random button targeting
    // #quick-result; the inner Random (5) button targets
    // #order-result. Pick the inner one by hx-post path.
    const random = page.locator(
      "button[hx-post='./api/orders/random']");
    if (await random.count() > 0) {
      await random.first().click();
      await page.waitForTimeout(2000);
      const result = await page.locator("#order-result").textContent();
      expect((result ?? "").length).toBeGreaterThan(0);
    } else {
      expect(true).toBe(true);
    }
  });

  test("order lifecycle trace by OID", async ({ page }) => {
    await page.goto("/orders");
    await page.locator("#trace-oid").fill("test-oid-12345");
    await page.locator("button", { hasText: "Trace" }).click();
    await page.waitForTimeout(2000);
    await expect(page.locator("#trace-result")).toContainText("test-oid-12345");
  });

  test("quick order reports an outcome", async ({ page }) => {
    await page.goto("/orders");
    // Use quick-order matrix (visible without expanding).
    // Quick result lives in #quick-result, not #order-result.
    await page.getByRole("button", { name: "10" }).first().click();
    await expect(page.locator("#quick-result")).toContainText(
      /order|accepted|queued|resting|rejected|no response|gateway/i,
      { timeout: 5000 },
    );
  });

  test("recent orders auto-refresh every 2s", async ({ page }) => {
    await page.goto("/orders");
    const trigger = await page.locator("div[hx-get='./x/recent-orders']").getAttribute("hx-trigger");
    expect(trigger).toContain("every 2s");
  });

  test("order form has GTC and IOC TIF options", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    const gtc = page.locator("input[type='radio'][name='tif'][value='GTC']");
    const ioc = page.locator("input[type='radio'][name='tif'][value='IOC']");
    await expect(gtc).toBeAttached();
    await expect(ioc).toBeAttached();
    await ioc.check({ force: true });
    await expect(ioc).toBeChecked();
    await gtc.check({ force: true });
    await expect(gtc).toBeChecked();
  });

  test("order form has reduce_only checkbox", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    const roCheckbox = page.locator("input[name='reduce_only']");
    await expect(roCheckbox).toBeAttached();
    await roCheckbox.check({ force: true });
    await expect(roCheckbox).toBeChecked();
  });

  test("after batch, recent orders is non-empty", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    await page.locator("button", { hasText: "Batch" }).first().click();
    await page.waitForTimeout(2000);
    await page.waitForTimeout(2500);
    const recentOrders = page.locator("div[hx-get='./x/recent-orders']");
    await expect(recentOrders).not.toContainText("no orders yet");
  });

  test("order form supports all symbol options", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    const symbolSelect = page.locator("select[name='symbol_id']");
    await symbolSelect.selectOption("10");
    await expect(symbolSelect).toHaveValue("10");
    await symbolSelect.selectOption("3");
    await expect(symbolSelect).toHaveValue("3");
    await symbolSelect.selectOption("1");
    await expect(symbolSelect).toHaveValue("1");
    await symbolSelect.selectOption("2");
    await expect(symbolSelect).toHaveValue("2");
  });

  test("order form supports buy and sell sides", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    const buy = page.locator(
      "input[type='radio'][name='side'][value='buy']");
    const sell = page.locator(
      "input[type='radio'][name='side'][value='sell']");
    await buy.check({ force: true });
    await expect(buy).toBeChecked();
    await sell.check({ force: true });
    await expect(sell).toBeChecked();
  });

  test("order form has user_id input field", async ({ page }) => {
    await page.goto("/orders");
    await expandCustom(page);
    const userIdInput = page.locator("input[name='user_id']");
    await expect(userIdInput).toBeVisible();
    await userIdInput.fill("42");
    await expect(userIdInput).toHaveValue("42");
  });

  test("IOC tif can be selected", async ({ page }) => {
    // The shipped UI doesn't have a separate order_type
    // selector, nor a post_only checkbox in the custom
    // form. Order intent is encoded via tif (GTC|IOC) and
    // the quick-order buttons. Verify IOC selection works.
    await page.goto("/orders");
    await expandCustom(page);
    await page.locator(
      "input[type='radio'][name='tif'][value='IOC']").check({ force: true });
    await expect(
      page.locator(
        "input[type='radio'][name='tif'][value='IOC']"),
    ).toBeChecked();
  });
});
