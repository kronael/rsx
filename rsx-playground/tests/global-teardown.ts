/**
 * Global teardown: stop the RSX processes global-setup started. They run
 * detached from the server, so closing the server no longer stops them.
 */

const BASE = "http://localhost:49171";

export default async function globalTeardown() {
  try {
    const res = await fetch(`${BASE}/api/processes/all/stop?confirm=yes`, {
      method: "POST",
      signal: AbortSignal.timeout(60_000),
    });
    const text = (await res.text()).replace(/<[^>]+>/g, "").trim();
    console.log("global-teardown:", text);
  } catch (e) {
    console.warn("global-teardown: stop failed:", e);
  }
}
