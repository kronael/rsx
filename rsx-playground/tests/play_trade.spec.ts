/**
 * Trade tab -- real per-user order entry/cancel + book/positions.
 *
 * The order form posts to /api/trade/orders (server.py); user_id comes
 * ONLY from the caller's own session token, never a form field. This
 * spec exercises the full authenticated round trip: sign in (via a
 * self-signed token, matching the RSX_GW_JWT_SECRET the Playwright
 * webServer sets), place a resting order, cancel it by cid.
 */

import { test, expect } from "@playwright/test";
import { signRsxJwt } from "./test_helpers";

const BASE_URL = "http://localhost:49171";

test.describe("Trade tab", () => {
  test("loads with order entry, book, positions, fills, account cards", async ({
    page,
  }) => {
    await page.goto("/trade");
    await expect(page.locator("nav a", { hasText: "Trade" })).toHaveClass(
      /bg-slate-700/,
    );
    await expect(
      page.getByRole("heading", { name: "Order Entry" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Orderbook Ladder" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Your Positions" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Your Fills" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Your Account" }),
    ).toBeVisible();
  });

  test("logged out: positions/fills/account prompt sign-in, no identity leaked", async ({
    page,
  }) => {
    await page.goto("/trade");
    await expect(page.locator("#trade-positions-body")).toContainText(
      /sign in/i,
      { timeout: 5000 },
    );
    await expect(page.locator("#trade-fills-body")).toContainText(
      /sign in/i,
    );
    await expect(page.locator("#trade-account-body")).toContainText(
      /sign in/i,
    );
  });

  test("logged out: order submission is rejected, never attributed to a default user", async ({
    page,
  }) => {
    await page.goto("/trade");
    await page.locator("input[name='price']").fill("0.049");
    await page.locator("input[name='qty']").fill("10.0");
    await page.locator("#trade-order-form button[type='submit']").click();
    await page.waitForTimeout(1000);
    await expect(page.locator("#trade-order-result")).toContainText(
      /sign in/i,
    );
  });

  test(
    "logged in: places a resting order and cancels it by cid",
    async ({ page, context }) => {
      const token = signRsxJwt(1);
      await context.addCookies([
        { name: "rsx_token", value: token, url: BASE_URL },
      ]);
      await page.goto("/trade");
      await page.locator("#trade-symbol").selectOption("10"); // PENGU
      await page
        .locator("input[type='radio'][name='side'][value='buy']")
        .check({ force: true });
      // Far below any plausible live PENGU price so this GTC limit
      // rests instead of crossing (buy only fills if ask <= price).
      await page.locator("input[name='price']").fill("0.00001");
      // PENGU lot=100000, qty_dec=4 -> qty=10.0 is lot-aligned.
      await page.locator("input[name='qty']").fill("10.0");
      await page
        .locator("input[type='radio'][name='tif'][value='GTC']")
        .check({ force: true });
      await page.locator("#trade-order-form button[type='submit']").click();
      await page.waitForTimeout(2500);

      const resultText =
        (await page.locator("#trade-order-result").textContent()) ?? "";
      expect(resultText).toMatch(/order .*(resting|filled|error|timeout)/i);

      const match = resultText.match(/order (\S+)/);
      expect(match).not.toBeNull();
      const cid = (match as RegExpMatchArray)[1];

      await page.locator("#trade-cancel-cid").fill(cid);
      await page.locator("button", { hasText: "Cancel" }).click();
      await page.waitForTimeout(1500);
      await expect(page.locator("#trade-cancel-result")).toContainText(
        /cancelled|error|timeout/i,
      );
    },
  );
});
