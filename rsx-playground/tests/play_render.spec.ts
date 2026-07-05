import { test, expect, Page } from "@playwright/test";
import { waitForHTMX } from "./test_helpers";

// ── Layer 2: RENDER INTEGRITY ───────────────────────────────
// The audit saw escaped tags render as literal text (`<abbr` /
// `&lt;`), raw JSON blobs printed where a widget belongs, and
// "unknown"/"—" in status slots that a 7/7 cluster should fill.
// A "page loads" assertion never caught any of it. This checks
// what the visitor actually READS.
//
// Locks FINDINGS #21 (raw JSON leak), the escaped-<abbr> class,
// #5/#12/#18 (unpopulated status slots), and the <base> prefix-
// safety invariant.

const NAV_PAGES = [
  "/overview", "/topology", "/book", "/risk", "/wal", "/logs",
  "/control", "/maker", "/faults", "/recovery", "/verify", "/orders",
  "/stress", "/cast", "/latency", "/crates", "/components",
];
const NESTED_PAGES = [
  "/crate/rsx-cast", "/crate/rsx-matching", "/crate/rsx-cli",
  "/component/gateway", "/component/risk", "/component/matching",
];

// Text-node markers of a double-escaped tag leaking into the body.
const ESCAPED_MARKUP = ["&lt;", "&gt;", "<abbr", "<span", "<div class"];
// Raw JSON that should have been formatted into a widget.
const JSON_LEAK = ['{"ok":true', '{"code":', '{"rand_', '{"status":"'];

async function bodyText(page: Page, path: string): Promise<string> {
  await page.goto(path);
  await waitForHTMX(page, 2500);
  return (await page.locator("body").innerText()) ?? "";
}

test.describe("Render integrity", () => {
  for (const path of [...NAV_PAGES, ...NESTED_PAGES]) {
    test(`${path}: no escaped markup or raw JSON leaks to body`,
      async ({ page }) => {
        const txt = await bodyText(page, path);
        for (const m of ESCAPED_MARKUP) {
          expect(txt, `${path} leaked escaped markup ${m}`)
            .not.toContain(m);
        }
        for (const j of JSON_LEAK) {
          expect(txt, `${path} leaked raw JSON ${j}`).not.toContain(j);
        }
      });
  }

  // (d) <base href> prefix-safety invariant: root pages ./, nested ../
  test("base href is ./ on root pages, ../ on nested pages",
    async ({ page }) => {
      for (const p of ["/overview", "/orders", "/wal"]) {
        await page.goto(p);
        expect(await page.locator("base").getAttribute("href"),
          `${p} base`).toBe("./");
      }
      for (const p of ["/crate/rsx-cast", "/component/gateway"]) {
        await page.goto(p);
        expect(await page.locator("base").getAttribute("href"),
          `${p} base`).toBe("../");
      }
    });

  // (c) Status/data slots must be populated on a live cluster — not
  // "unknown". Scoped to the specific slots the audit flagged (a
  // whole-body scan would false-positive on legit "UNKNOWN(N)" doc
  // copy on the CLI crate page).
  test("status slots are populated (not unknown) on a live cluster",
    async ({ page, request }) => {
      const health = await request.get("/healthz");
      test.skip(!health.ok(), "cluster unreachable");
      const j = await health.json();
      test.skip((j.processes_running ?? 0) < 6, "cluster not up (need 6+)");

      await page.goto("/overview");
      await waitForHTMX(page, 2500);

      // proc-chip: "procs N/N", never "unknown".
      const chip = await page.locator("#proc-chip").innerText();
      expect(chip.toLowerCase()).not.toContain("unknown");
      expect(chip).toMatch(/\d+\/\d+/);

      // invariant status must resolve to a verdict, not "unknown".
      const inv = await (await request.get("/x/invariant-status")).text();
      expect(inv.toLowerCase()).not.toContain("unknown");
      expect(inv).toMatch(/passing|violat|check/i);

      // health gauge is a numeric score with a colour word (FINDING
      // #12: was YELLOW/-25 at 7/7 with no findable panic).
      const gauge = await (await request.get("/x/health")).text();
      expect(gauge).toMatch(/\d+/);
      expect(gauge.toLowerCase()).not.toContain("unknown");
    });
});
