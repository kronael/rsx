/**
 * Users tab -- admin-only user + role management.
 *
 * /users is gated on the caller's own LIVE role claim, re-checked
 * against rsx-auth on every load (server.py `_caller_role`) -- never
 * a cached/local decision, and never the loopback/PLAYGROUND_ADMIN_TOKEN
 * gate the ops harness uses. This environment has no live rsx-auth
 * deployed (see play_login.spec.ts), so a real "admin sees the table"
 * round trip isn't reachable here; that success path is covered by
 * api_users_test.py's mocked _caller_role. This spec verifies the
 * fail-closed behavior a real browser experiences: the tab is visible
 * in nav, but its content is refused without a live admin role --
 * never "trust anyway" when rsx-auth can't be reached.
 */

import { test, expect } from "@playwright/test";
import { signRsxJwt } from "./test_helpers";

const BASE_URL = "http://localhost:49171";

test.describe("Users tab", () => {
  test("nav shows the Users tab link", async ({ page }) => {
    await page.goto("/overview");
    await expect(
      page.getByRole("navigation").getByRole("link", { name: "Users" }),
    ).toBeVisible();
  });

  test("logged out: admin access required, no user data leaked", async ({
    page,
  }) => {
    const resp = await page.goto("/users");
    expect(resp?.status()).toBe(403);
    await expect(page.getByText(/admin access required/i)).toBeVisible();
  });

  test(
    "logged in without a live admin role: still refused (fail closed)",
    async ({ page, context }) => {
      // A valid session token alone is not enough -- role is re-checked
      // live against rsx-auth on every load, and this environment has
      // no rsx-auth deployed, so the caller never resolves to admin.
      const token = signRsxJwt(1);
      await context.addCookies([
        { name: "rsx_token", value: token, url: BASE_URL },
      ]);
      const resp = await page.goto("/users");
      expect(resp?.status()).toBe(403);
      await expect(page.getByText(/admin access required/i)).toBeVisible();
    },
  );
});
