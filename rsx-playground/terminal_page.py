"""Terminal page for the embedded rsx-term view."""

import pages


def page() -> str:
    content = '''
<link rel="stylesheet"
  href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.css">
<script src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.js"></script>
<script
  src="https://cdn.jsdelivr.net/npm/xterm-addon-fit@0.8.0/lib/xterm-addon-fit.js">
</script>
<div class="space-y-3">
  <div class="flex flex-wrap items-center justify-between gap-2">
    <div>
      <h2 class="text-sm font-semibold text-slate-200">rsx-term</h2>
      <p class="text-xs text-slate-500">
        Local terminal client against the Playground gateway and marketdata.
      </p>
    </div>
    <div id="term-status" class="text-xs text-slate-500">connecting...</div>
  </div>
  <div id="terminal"
    class="border border-slate-800 bg-black rounded overflow-hidden"
    style="height: min(72vh, 760px);"></div>
</div>
<script>
(function () {
  const el = document.getElementById("terminal");
  const status = document.getElementById("term-status");
  if (!window.Terminal) {
    status.textContent = "xterm.js failed to load";
    return;
  }
  const term = new Terminal({
    cursorBlink: true,
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
    fontSize: 13,
    theme: {
      background: "#020617",
      foreground: "#d6e0eb",
      cursor: "#e2c16b",
      selectionBackground: "#334155"
    }
  });
  const fit = window.FitAddon ? new FitAddon.FitAddon() : null;
  if (fit) term.loadAddon(fit);
  term.open(el);
  if (fit) fit.fit();

  const url = new URL("./ws/terminal", window.location.href);
  url.protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(url);

  function resize() {
    if (fit) fit.fit();
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({
        type: "resize",
        cols: term.cols,
        rows: term.rows
      }));
    }
  }

  ws.addEventListener("open", () => {
    status.textContent = "connected";
    resize();
  });
  ws.addEventListener("message", (event) => {
    term.write(event.data);
  });
  ws.addEventListener("close", () => {
    status.textContent = "closed";
  });
  ws.addEventListener("error", () => {
    status.textContent = "connection error";
  });
  term.onData((data) => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({type: "input", data}));
    }
  });
  window.addEventListener("resize", resize);
})();
</script>'''
    return pages.layout("Terminal", content, "./terminal")
