// Canvas chart primitives, DPR-aware, tuned for 10 Hz streaming data
// with a neon/cyberpunk rendering style.

export function sizeCanvas(canvas: HTMLCanvasElement, heightPx?: number) {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || canvas.parentElement?.clientWidth || 300;
  const h = heightPx ?? canvas.clientHeight ?? 120;
  if (heightPx && canvas.style.height !== `${heightPx}px`) canvas.style.height = `${heightPx}px`;
  const tw = Math.max(1, Math.round(w * dpr));
  const th = Math.max(1, Math.round(h * dpr));
  // reassigning width/height resets the GPU buffer — only do it on real resize
  if (canvas.width !== tw) canvas.width = tw;
  if (canvas.height !== th) canvas.height = th;
  const ctx = canvas.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { ctx, w, h };
}

class Ring {
  buf: Float32Array;
  len = 0;
  head = 0;
  constructor(cap: number) {
    this.buf = new Float32Array(cap);
  }
  push(v: number) {
    this.buf[this.head] = v;
    this.head = (this.head + 1) % this.buf.length;
    if (this.len < this.buf.length) this.len++;
  }
  at(i: number): number {
    // i = 0 → oldest
    const start = (this.head - this.len + this.buf.length) % this.buf.length;
    return this.buf[(start + i) % this.buf.length];
  }
  max(): number {
    let m = 0;
    for (let i = 0; i < this.len; i++) m = Math.max(m, this.at(i));
    return m;
  }
}

export interface SeriesSpec {
  color: string;
  fill?: boolean;
  width?: number;
}

export interface StripOpts {
  series: SeriesSpec[];
  min?: number;
  max?: number | "auto";
  capacity?: number;
  height?: number;
  gridSteps?: number;
}

export class StripChart {
  private canvas: HTMLCanvasElement;
  private opts: Required<StripOpts>;
  private rings: Ring[];
  private ro: ResizeObserver;

  constructor(canvas: HTMLCanvasElement, opts: StripOpts) {
    this.canvas = canvas;
    this.opts = {
      min: 0,
      max: 100,
      capacity: 600,
      height: 120,
      gridSteps: 4,
      ...opts,
    } as Required<StripOpts>;
    this.rings = opts.series.map(() => new Ring(this.opts.capacity));
    this.ro = new ResizeObserver(() => this.draw());
    this.ro.observe(canvas);
  }

  destroy() {
    this.ro.disconnect();
  }

  push(values: number[]) {
    values.forEach((v, i) => this.rings[i]?.push(isFinite(v) ? v : 0));
    this.draw();
  }

  draw() {
    // Hidden tab: data keeps accumulating in the rings, but skip the
    // canvas work — it's the expensive part at 10 Hz.
    if (this.canvas.offsetParent === null) return;
    const { ctx, w, h } = sizeCanvas(this.canvas, this.opts.height);
    ctx.clearRect(0, 0, w, h);

    const isWireframe = document.body.classList.contains("theme-wireframe");

    let max: number;
    if (this.opts.max === "auto") {
      max = Math.max(...this.rings.map((r) => r.max())) * 1.15;
      if (max <= 0) max = 1;
      // snap to a pleasant ceiling
      const pow = Math.pow(10, Math.floor(Math.log10(max)));
      max = Math.ceil(max / pow) * pow;
    } else {
      max = this.opts.max as number;
    }
    const min = this.opts.min;

    // grid
    const gridAlpha = isWireframe ? 0.04 : 0.07;
    ctx.strokeStyle = `rgba(0,229,255,${gridAlpha})`;
    ctx.lineWidth = 1;
    for (let g = 1; g < this.opts.gridSteps; g++) {
      const y = (h * g) / this.opts.gridSteps;
      ctx.beginPath();
      ctx.moveTo(0, y + 0.5);
      ctx.lineTo(w, y + 0.5);
      ctx.stroke();
    }
    // vertical ticks every ~10s (100 samples)
    const cap = this.opts.capacity;
    for (let s = 100; s < cap; s += 100) {
      const x = w - (s / cap) * w;
      ctx.beginPath();
      ctx.moveTo(x + 0.5, 0);
      ctx.lineTo(x + 0.5, h);
      ctx.stroke();
    }

    const yOf = (v: number) => h - ((v - min) / (max - min)) * (h - 4) - 2;

    this.rings.forEach((ring, si) => {
      if (ring.len < 2) return;
      const spec = this.opts.series[si];
      const step = w / (cap - 1);
      const x0 = w - (ring.len - 1) * step;

      ctx.beginPath();
      for (let i = 0; i < ring.len; i++) {
        const x = x0 + i * step;
        const y = yOf(ring.at(i));
        i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
      }

      if (spec.fill && !isWireframe) {
        const grad = ctx.createLinearGradient(0, 0, 0, h);
        grad.addColorStop(0, spec.color + "30");
        grad.addColorStop(1, spec.color + "02");
        ctx.save();
        ctx.lineTo(x0 + (ring.len - 1) * step, h);
        ctx.lineTo(x0, h);
        ctx.closePath();
        ctx.fillStyle = grad;
        ctx.fill();
        ctx.restore();
        // re-trace the line path (fill consumed it)
        ctx.beginPath();
        for (let i = 0; i < ring.len; i++) {
          const x = x0 + i * step;
          const y = yOf(ring.at(i));
          i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
        }
      } else if (spec.fill && isWireframe) {
        // wireframe: very faint fill with dashed effect
        const grad = ctx.createLinearGradient(0, 0, 0, h);
        grad.addColorStop(0, spec.color + "12");
        grad.addColorStop(1, spec.color + "01");
        ctx.save();
        ctx.lineTo(x0 + (ring.len - 1) * step, h);
        ctx.lineTo(x0, h);
        ctx.closePath();
        ctx.fillStyle = grad;
        ctx.fill();
        ctx.restore();
        ctx.beginPath();
        for (let i = 0; i < ring.len; i++) {
          const x = x0 + i * step;
          const y = yOf(ring.at(i));
          i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
        }
      }

      // cheap two-pass glow (shadowBlur at 10 Hz is too costly in WebKit)
      const glowAlpha = isWireframe ? "22" : "3a";
      ctx.strokeStyle = spec.color + glowAlpha;
      ctx.lineWidth = isWireframe ? (spec.width ?? 1.2) + 1.5 : (spec.width ?? 1.6) + 3;
      ctx.stroke();
      ctx.strokeStyle = spec.color;
      ctx.lineWidth = isWireframe ? (spec.width ?? 1.0) : (spec.width ?? 1.6);
      ctx.stroke();

      // head dot with halo
      const hx = x0 + (ring.len - 1) * step;
      const hy = yOf(ring.at(ring.len - 1));
      ctx.beginPath();
      ctx.arc(hx, hy, 5, 0, Math.PI * 2);
      ctx.fillStyle = spec.color + "40";
      ctx.fill();
      ctx.beginPath();
      ctx.arc(hx, hy, 2.2, 0, Math.PI * 2);
      ctx.fillStyle = "#fff";
      ctx.fill();
    });

    // max label
    ctx.fillStyle = "rgba(134,174,196,0.55)";
    ctx.font = "10px 'JetBrains Mono Variable', monospace";
    ctx.textAlign = "left";
    ctx.fillText(this.fmtAxis(max), 4, 11);
  }

  private fmtAxis(v: number): string {
    if (v >= 1e9) return (v / 1e9).toFixed(0) + "G";
    if (v >= 1e6) return (v / 1e6).toFixed(0) + "M";
    if (v >= 1e3) return (v / 1e3).toFixed(0) + "K";
    return String(Math.round(v));
  }
}

// ---------------- radial gauge ----------------

export class RingGauge {
  private canvas: HTMLCanvasElement;
  private label: string;
  private unit: string;
  private max: number;
  private value = 0;
  private shown = 0; // eased
  private raf = 0;
  private size: number;

  constructor(canvas: HTMLCanvasElement, label: string, unit: string, max = 100, size = 108) {
    this.canvas = canvas;
    this.label = label;
    this.unit = unit;
    this.max = max;
    this.size = size;
    canvas.style.width = `${size}px`;
    canvas.style.height = `${size}px`;
  }

  set(v: number) {
    this.value = isFinite(v) ? v : 0;
    if (!this.raf) this.tick();
  }

  private tick = () => {
    this.shown += (this.value - this.shown) * 0.25;
    this.draw();
    if (Math.abs(this.value - this.shown) > 0.1) {
      this.raf = requestAnimationFrame(this.tick);
    } else {
      this.raf = 0;
    }
  };

  private color(frac: number): string {
    if (frac > 0.86) return "#ff3b5c";
    if (frac > 0.7) return "#ffb02e";
    return "#00e5ff";
  }

  draw() {
    if (this.canvas.offsetParent === null) return;
    const dpr = window.devicePixelRatio || 1;
    const s = this.size;
    if (this.canvas.width !== s * dpr) {
      this.canvas.width = s * dpr;
      this.canvas.height = s * dpr;
    }
    const ctx = this.canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, s, s);

    const cx = s / 2;
    const cy = s / 2;
    const r = s / 2 - 9;
    const start = Math.PI * 0.75;
    const span = Math.PI * 1.5;
    const frac = Math.min(1, Math.max(0, this.shown / this.max));
    const col = this.color(frac);

    const isDrac = document.body.classList.contains("theme-dracula");
    ctx.lineCap = "round";
    ctx.beginPath();
    ctx.arc(cx, cy, r, start, start + span);
    ctx.strokeStyle = isDrac ? "rgba(98,114,164,0.18)" : "rgba(0,229,255,0.1)";
    ctx.lineWidth = 5;
    ctx.stroke();

    if (frac > 0.005) {
      ctx.beginPath();
      ctx.arc(cx, cy, r, start, start + span * frac);
      ctx.strokeStyle = col;
      ctx.lineWidth = 5;
      ctx.shadowColor = col;
      ctx.shadowBlur = 10;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }

    ctx.fillStyle = "#d9f4ff";
    ctx.font = "600 19px 'JetBrains Mono Variable', monospace";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(Math.round(this.shown) + "", cx, cy - 4);
    ctx.fillStyle = "rgba(134,174,196,0.8)";
    ctx.font = "10px 'JetBrains Mono Variable', monospace";
    ctx.fillText(this.unit, cx, cy + 13);
    ctx.fillStyle = "rgba(74,107,128,0.9)";
    ctx.font = "600 8.5px Orbitron, sans-serif";
    ctx.fillText(this.label.toUpperCase(), cx, s - 7);
  }
}
