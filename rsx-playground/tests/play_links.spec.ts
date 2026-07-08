import { test, expect, Page, APIRequestContext } from "@playwright/test";

// ── Layer 1: LINK INTEGRITY ─────────────────────────────────
// The old suite asserted a page *renders*; it never followed the
// links that page emits. So a `../component` that escaped the app
// root, a missing /crate/* page (404), a broken demo-GIF src, or a
// doc-link 404 all sailed through green. This test crawls every
// page's outbound links (href / hx-get / hx-post / img src),
// resolves each against the page's own <base>, and probes it.
//
// Locks in the missing-crates + md-404 + broken-region-link class
// (FINDINGS #2-adjacent, #30 crate pages, docs links).

// Static nav pages (the tabs + the index pages the nav links to).
const NAV_PAGES = [
  "/", "/overview", "/topology", "/book", "/risk", "/wal", "/logs",
  "/control", "/maker", "/faults", "/recovery", "/verify", "/orders",
  "/stress", "/terminal", "/cast", "/latency", "/docs", "/crates",
  "/components",
];

interface Link {
  url: string;   // absolute, resolved against document.baseURI
  method: string;
  raw: string;
}

// Collect every internal link the browser sees on `path` (after
// <base> resolution). External http(s) hosts, #anchors, data:,
// mailto:, javascript: are dropped.
async function collectLinks(page: Page, path: string): Promise<Link[]> {
  await page.goto(path);
  const origin = new URL(page.url()).origin;
  const raw = await page.evaluate(() => {
    const out: { raw: string | null; method: string }[] = [];
    const add = (r: string | null, m: string) => out.push({ raw: r, method: m });
    document.querySelectorAll("[href]").forEach((e) =>
      add(e.getAttribute("href"), "GET"));
    document.querySelectorAll("[hx-get]").forEach((e) =>
      add(e.getAttribute("hx-get"), "GET"));
    document.querySelectorAll("[hx-post]").forEach((e) =>
      add(e.getAttribute("hx-post"), "POST"));
    document.querySelectorAll("img[src]").forEach((e) =>
      add(e.getAttribute("src"), "GET"));
    return out.map((o) => {
      let url: string | null = null;
      try {
        url = o.raw ? new URL(o.raw, document.baseURI).href : null;
      } catch { url = null; }
      return { url, method: o.method, raw: o.raw ?? "" };
    });
  });
  const links: Link[] = [];
  for (const l of raw) {
    if (!l.url) continue;
    if (!l.url.startsWith(origin)) continue;              // external
    const r = l.raw.trim();
    if (r === "" || r.startsWith("#") || r.startsWith("mailto:") ||
        r.startsWith("data:") || r.startsWith("javascript:")) continue;
    // strip a trailing #fragment before probing
    links.push({ url: l.url.split("#")[0], method: l.method, raw: r });
  }
  return links;
}

// Enumerate the concrete /crate/* and /component/* detail pages
// from their index pages (14 crates, 7 components today).
async function detailPages(page: Page): Promise<string[]> {
  const out: string[] = [];
  for (const [idx, sel] of [
    ["/crates", "a[href*='/crate/']"],
    ["/components", "a[href*='/component/']"],
  ] as const) {
    await page.goto(idx);
    const hrefs = await page.$$eval(sel, (els) =>
      els.map((e) => (e as HTMLAnchorElement).href));
    for (const h of hrefs) out.push(new URL(h).pathname);
  }
  return Array.from(new Set(out));
}

async function probe(
  req: APIRequestContext, l: Link,
): Promise<string | null> {
  // GET-probe everything (never POST — avoid side effects). A GET on a
  // POST-only route is 405 in FastAPI, which proves the route EXISTS
  // (not a 404). So: GET links must be <400; hx-post routes must not
  // be 404/5xx (405 is the healthy "exists but POST-only" answer).
  const resp = await req.get(l.url, { maxRedirects: 0 });
  const s = resp.status();
  if (l.method === "POST") {
    if (s === 405 || (s >= 200 && s < 400)) return null;
    return `POST-route ${l.raw} -> ${s} (expected 405/2xx/3xx; 404 means missing route)`;
  }
  if (s >= 200 && s < 400) return null;
  return `GET ${l.raw} -> ${s}`;
}

test.describe("Link integrity", () => {
  test.describe.configure({ timeout: 120_000 });

  test("every internal link/hx-endpoint/img resolves (no 404 escapes)",
    async ({ page, request }) => {
      const pages = [...NAV_PAGES, ...(await detailPages(page))];
      // Detail pages themselves must exist (locks the 7-missing-crates
      // + missing-component-page class directly).
      const detail = pages.filter((p) =>
        p.startsWith("/crate/") || p.startsWith("/component/"));
      expect(detail.length).toBeGreaterThanOrEqual(14 + 7);

      // Collect a de-duplicated (url, method) work set across all pages.
      const seen = new Map<string, Link>();
      for (const p of pages) {
        for (const l of await collectLinks(page, p)) {
          seen.set(`${l.method} ${l.url}`, l);
        }
      }
      expect(seen.size).toBeGreaterThan(30);

      // At least one demo-GIF img must be in the set (else the
      // broken-GIF-src assertion below is vacuous). Demo GIFs are
      // served extensionless via /x/crate-demo/{crate}; a real image
      // src ending in .gif/.png also counts.
      const gifs = [...seen.values()].filter((l) =>
        /\/x\/crate-demo\//.test(l.url) ||
        /\.(gif|png|svg|webp)$/i.test(l.url));
      expect(gifs.length).toBeGreaterThan(0);

      const failures: string[] = [];
      for (const l of seen.values()) {
        const f = await probe(request, l);
        if (f) failures.push(f);
      }
      expect(failures, `broken links:\n${failures.join("\n")}`).toEqual([]);
    });

  // Direct, named guard for the missing-crate-page regression: each of
  // the 14 crate pages must be 200 (not 404).
  test("all 14 crate pages return 200", async ({ page, request }) => {
    await page.goto("/crates");
    const paths = await page.$$eval("a[href*='/crate/']", (els) =>
      els.map((e) => new URL((e as HTMLAnchorElement).href).pathname));
    const uniq = Array.from(new Set(paths));
    expect(uniq.length).toBe(14);
    for (const p of uniq) {
      const r = await request.get(p);
      expect(r.status(), `${p}`).toBe(200);
    }
  });
});
