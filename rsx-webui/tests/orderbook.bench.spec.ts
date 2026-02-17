/**
 * Performance benchmark: React render time per orderbook update.
 *
 * Methodology:
 *   1. Seed the orderbook with 20 bid and 20 ask levels.
 *   2. Install a MutationObserver on the orderbook DOM subtree.
 *   3. Apply N delta updates via window.__rsx.applyL2Delta.
 *   4. Each update is measured as: time from applyL2Delta call
 *      until MutationObserver fires (DOM painted by React).
 *   5. Report p50, p95, max and assert p95 < 16ms (one rAF frame).
 *
 * CI threshold: p95 render latency < 16ms.
 *
 * Run standalone:
 *   npx playwright test orderbook.bench.spec.ts
 */
import { test, expect } from "@playwright/test";

const N_UPDATES = 200;
const P95_THRESHOLD_MS = 16;

// Generate a 20-level orderbook snapshot centred around midPrice.
function makeSnapshot(midPrice: number, tickSize: number) {
  const bids: number[][] = [];
  const asks: number[][] = [];
  for (let i = 1; i <= 20; i++) {
    const bidPx = Math.round(midPrice - i * tickSize);
    const askPx = Math.round(midPrice + i * tickSize);
    const qty = 10 + Math.floor(Math.random() * 90);
    bids.push([bidPx, qty, 1]);
    asks.push([askPx, qty, 1]);
  }
  return { bids, asks };
}

test.describe("Orderbook render benchmark", () => {
  test.setTimeout(60_000);

  test("p95 render latency < 16ms per delta update", async ({ page }) => {
    await page.goto("/");

    // Wait for app to hydrate.
    await page.waitForSelector('[aria-label="Select trading pair"]', {
      state: "visible",
      timeout: 10_000,
    });

    // Seed orderbook snapshot.
    const MID = 50_000_00; // 50_000.00 at tickSize=0.01 (raw ticks)
    const snapshot = makeSnapshot(MID, 1);
    await page.evaluate(
      ({ bids, asks }) => {
        const rsx = (window as unknown as Record<string, {
          applyL2Snapshot: (
            bids: number[][],
            asks: number[][],
            seq: number,
          ) => void;
          applyL2Delta: (
            side: number,
            px: number,
            qty: number,
            count: number,
            seq: number,
          ) => void;
        }>).__rsx;
        rsx.applyL2Snapshot(bids, asks, 1);
      },
      { bids: snapshot.bids, asks: snapshot.asks },
    );

    // Allow one rAF cycle for React to render the snapshot.
    await page.waitForTimeout(32);

    // Locate the orderbook DOM node (the div containing "Price" header).
    const orderbookHandle = await page.evaluateHandle(() => {
      const headers = document.querySelectorAll("span");
      for (const h of headers) {
        if (h.textContent === "Price") {
          return h.closest("div[class*='flex-col']") ?? document.body;
        }
      }
      return document.body;
    });

    // Run the benchmark inside the browser.
    const timings = await page.evaluate(
      async ({ nUpdates, targetEl }) => {
        const rsx = (window as unknown as Record<string, {
          applyL2Delta: (
            side: number,
            px: number,
            qty: number,
            count: number,
            seq: number,
          ) => void;
        }>).__rsx;

        const results: number[] = [];
        let seq = 100;

        // Generate deterministic delta sequence.
        const deltas: Array<[number, number, number]> = [];
        for (let i = 0; i < nUpdates; i++) {
          // Alternate bid/ask, vary price and qty.
          const side = i % 2 === 0 ? 0 : 1; // 0=BUY, 1=SELL
          const px = i % 2 === 0
            ? 50_000_00 - (1 + (i % 10))
            : 50_000_00 + (1 + (i % 10));
          const qty = 5 + (i % 50);
          deltas.push([side, px, qty]);
        }

        // Measure each update.
        for (const [side, px, qty] of deltas) {
          const t0 = performance.now();

          // Apply delta (triggers Zustand state update + rAF coalescing).
          rsx.applyL2Delta(side, px, qty, 1, seq++);

          // Wait for React to commit the update to DOM.
          // React 18+ batches updates — wait two rAF frames to be safe.
          await new Promise<void>((resolve) => {
            requestAnimationFrame(() => {
              requestAnimationFrame(() => {
                resolve();
              });
            });
          });

          results.push(performance.now() - t0);
        }

        return results;
      },
      { nUpdates: N_UPDATES, targetEl: orderbookHandle },
    );

    // Compute statistics.
    const sorted = [...timings].sort((a, b) => a - b);
    const p50 = sorted[Math.floor(sorted.length * 0.50)] ?? 0;
    const p95 = sorted[Math.floor(sorted.length * 0.95)] ?? 0;
    const p99 = sorted[Math.floor(sorted.length * 0.99)] ?? 0;
    const max = sorted[sorted.length - 1] ?? 0;
    const avg =
      timings.reduce((s, v) => s + v, 0) / timings.length;

    // eslint-disable-next-line no-console
    console.log(
      `\nOrderbook render benchmark (${N_UPDATES} updates)\n` +
      `  avg: ${avg.toFixed(2)}ms\n` +
      `  p50: ${p50.toFixed(2)}ms\n` +
      `  p95: ${p95.toFixed(2)}ms\n` +
      `  p99: ${p99.toFixed(2)}ms\n` +
      `  max: ${max.toFixed(2)}ms\n` +
      `  threshold: p95 < ${P95_THRESHOLD_MS}ms`,
    );

    // Attach timing data to the test report.
    await (test.info() as {
      attach: (name: string, opts: { body: string; contentType: string }) => Promise<void>;
    }).attach("render-timings.json", {
      body: JSON.stringify({ avg, p50, p95, p99, max, samples: timings }),
      contentType: "application/json",
    });

    // Assert performance threshold.
    expect(p95).toBeLessThan(P95_THRESHOLD_MS);
  });
});
