/**
 * Login affordance -- nav identity chip (#session-chip).
 *
 * Logged-out shows a GitHub login link; logged-in shows the caller's OWN
 * resolved identity (from their own token, never a spoofed/admin-minted
 * user) plus a log-out control. See rsx-playground/server.py
 * `x_session_chip` / `_resolve_request_user`.
 */

import { test, expect } from "@playwright/test";
import { signRsxJwt } from "./test_helpers";

test.describe("Login affordance", () => {
  test("logged out: nav shows a GitHub login link", async ({ page }) => {
    await page.goto("/overview");
    const chip = page.locator("#session-chip");
    await expect(chip).toBeVisible();
    await expect(chip).toHaveText(/log in with GitHub/);
    await expect(chip).toHaveAttribute("href", /oauth\/github\/login/);
  });

  test(
    "logged in: nav shows the caller's own identity + log out",
    async ({ page, context }) => {
      const token = signRsxJwt(4242);
      await context.addCookies([
        { name: "rsx_token", value: token, url: "http://localhost:49171" },
      ]);
      await page.goto("/overview");
      const chip = page.locator("#session-chip");
      await expect(chip).toBeVisible();
      // rsx-auth may not be reachable in this environment; the chip still
      // resolves the caller's own user_id from their own token and falls
      // back to a bare label -- it never shows a different user.
      await expect(chip).toContainText("user 4242");
      await expect(
        chip.getByRole("button", { name: "log out" }),
      ).toBeVisible();
    },
  );

  test(
    "log out clears the session and reverts to the login link",
    async ({ page, context }) => {
      const token = signRsxJwt(4242);
      await context.addCookies([
        { name: "rsx_token", value: token, url: "http://localhost:49171" },
      ]);
      // Mock rsx-auth's own logout response (Set-Cookie clears rsx_token)
      // so this test exercises the client affordance regardless of
      // whether a live rsx-auth is deployed in this environment.
      await page.route("**/auth/logout", (route) =>
        route.fulfill({
          status: 200,
          headers: { "set-cookie": "rsx_token=; Max-Age=0; Path=/" },
          contentType: "application/json",
          body: '{"ok":true}',
        }),
      );
      await page.goto("/overview");
      await page
        .locator("#session-chip")
        .getByRole("button", { name: "log out" })
        .click();
      await page.waitForLoadState("networkidle");
      const chip = page.locator("#session-chip");
      await expect(chip).toHaveText(/log in with GitHub/);
    },
  );
});
