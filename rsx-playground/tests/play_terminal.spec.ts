import { test, expect, type Page } from "@playwright/test";

async function expectTerminalFocused(page: Page) {
  const active = await page.evaluate(() => {
    const el = document.activeElement;
    return {
      tag: el?.tagName ?? "",
      cls: String(el?.className ?? ""),
      inXterm: Boolean(el?.closest?.(".xterm")),
    };
  });
  expect(active.inXterm, JSON.stringify(active)).toBe(true);
}

test.describe("Embedded rsx-term terminal", () => {
  test("connects, renders rsx-term, and captures terminal keys",
    async ({ page }) => {
      const wsPromise = page.waitForEvent(
        "websocket",
        (ws) => ws.url().endsWith("/ws/terminal"),
      );
      await page.goto("/terminal");
      const ws = await wsPromise;
      expect(ws.url()).toMatch(/\/ws\/terminal$/);

      await expect(
        page.getByRole("heading", { name: "rsx-term" })
      ).toBeVisible();
      await expect(page.locator("#term-status")).toHaveText(
        "connected",
        { timeout: 12_000 },
      );
      await expect(page.locator(".xterm")).toHaveCount(1);

      const terminal = page.locator("#terminal");
      await expect(terminal).toContainText("PENGU-PERP", {
        timeout: 12_000,
      });
      const text = await terminal.innerText();
      expect(text).not.toMatch(/go not found|connection refused|exit status/i);

      await terminal.click();
      await expectTerminalFocused(page);

      for (const key of [
        "Tab",
        "Control+F",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "PageUp",
        "PageDown",
        "Home",
        "End",
        "F3",
        "Escape",
      ]) {
        await page.keyboard.press(key);
        await expectTerminalFocused(page);
        await expect(page).toHaveURL(/\/terminal$/);
        await expect(page.locator(".xterm")).toBeVisible();
      }
    },
  );
});
