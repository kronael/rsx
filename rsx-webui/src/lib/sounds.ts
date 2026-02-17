// Web Audio API sound generator — no audio files needed.
// AudioContext is lazy-initialized on first call.

let ctx: AudioContext | null = null;

function getCtx(): AudioContext | null {
  if (ctx) return ctx;
  try {
    ctx = new AudioContext();
  } catch {
    return null;
  }
  return ctx;
}

function resumeCtx(ac: AudioContext): Promise<void> {
  if (ac.state === "suspended") return ac.resume();
  return Promise.resolve();
}

function beep(
  ac: AudioContext,
  freq: number,
  startAt: number,
  duration: number,
  gain: number,
): void {
  const osc = ac.createOscillator();
  const g = ac.createGain();
  osc.connect(g);
  g.connect(ac.destination);

  osc.type = "sine";
  osc.frequency.setValueAtTime(freq, startAt);

  g.gain.setValueAtTime(0, startAt);
  g.gain.linearRampToValueAtTime(gain, startAt + 0.005);
  g.gain.exponentialRampToValueAtTime(0.0001, startAt + duration);

  osc.start(startAt);
  osc.stop(startAt + duration + 0.01);
}

// Short beep: buy = higher pitch (880 Hz), sell = lower pitch (440 Hz).
export function playFillSound(side: "buy" | "sell"): void {
  const ac = getCtx();
  if (!ac) return;

  resumeCtx(ac).then(() => {
    if (ac.state !== "running") return;
    const freq = side === "buy" ? 880 : 440;
    beep(ac, freq, ac.currentTime, 0.12, 0.18);
  });
}

// Urgent 3-pulse warning tone at 330 Hz with 150ms spacing.
export function playLiquidationSound(): void {
  const ac = getCtx();
  if (!ac) return;

  resumeCtx(ac).then(() => {
    if (ac.state !== "running") return;
    const now = ac.currentTime;
    beep(ac, 330, now, 0.1, 0.25);
    beep(ac, 330, now + 0.15, 0.1, 0.25);
    beep(ac, 330, now + 0.30, 0.1, 0.25);
  });
}
