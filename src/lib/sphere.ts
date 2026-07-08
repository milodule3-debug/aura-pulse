// The Global Sphere: a rotating fibonacci point-sphere with orbiting
// satellites, ping ripples and a load-reactive breathing glow. Reacts to
// live app activity: AI calls light an uplink ring, vault captures burst.

import { activity, latest } from "./bridge";

interface P3 {
  x: number;
  y: number;
  z: number;
}

interface Ping {
  idx: number;
  t: number; // 0..1
}

interface Satellite {
  angle: number;
  incl: number;
  speed: number;
  pingT: number;
}

export class GlobalSphere {
  private canvas: HTMLCanvasElement;
  private pts: P3[] = [];
  private rotY = 0;
  private tilt = 0.42;
  private pings: Ping[] = [];
  private sats: Satellite[] = [];
  private raf = 0;
  private lastPing = 0;
  private lastVaultSeen = 0;
  private t0 = performance.now();

  private lastFrame = 0;

  constructor(canvas: HTMLCanvasElement, n = 320) {
    this.canvas = canvas;
    // fibonacci sphere distribution
    const golden = Math.PI * (3 - Math.sqrt(5));
    for (let i = 0; i < n; i++) {
      const y = 1 - (i / (n - 1)) * 2;
      const r = Math.sqrt(1 - y * y);
      const th = golden * i;
      this.pts.push({ x: Math.cos(th) * r, y, z: Math.sin(th) * r });
    }
    this.sats = [
      { angle: 0, incl: 0.5, speed: 0.35, pingT: 0 },
      { angle: 2.1, incl: -0.35, speed: 0.52, pingT: 0.4 },
      { angle: 4.4, incl: 0.15, speed: 0.27, pingT: 0.7 },
    ];
    this.start();
  }

  start() {
    if (!this.raf) this.loop();
  }
  stop() {
    cancelAnimationFrame(this.raf);
    this.raf = 0;
  }

  private loop = () => {
    this.raf = requestAnimationFrame(this.loop);
    // 30 fps is plenty for the sphere and halves software-canvas cost
    const now = performance.now();
    if (now - this.lastFrame < 31) return;
    this.lastFrame = now;
    this.draw();
  };

  private draw() {
    const dpr = window.devicePixelRatio || 1;
    const size = this.canvas.clientWidth || 260;
    if (this.canvas.width !== size * dpr) {
      this.canvas.width = size * dpr;
      this.canvas.height = size * dpr;
    }
    const ctx = this.canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size, size);

    const t = (performance.now() - this.t0) / 1000;
    const cpu = latest.snap?.cpu.total ?? 12;
    const load = Math.min(1, cpu / 100);

    const isDrac = document.body.classList.contains("theme-dracula");
    const isWire = document.body.classList.contains("theme-wireframe");

    // rotation speed + breathing react to CPU load
    this.rotY += 0.0035 + load * 0.014;
    const breathe = 0.55 + 0.45 * Math.sin(t * (0.7 + load * 2.2));
    const hue = isDrac ? 190 - load * 15 : 188 - load * 30; // theme-adapted hue
    const satHue = isDrac ? 326 : 330; // satellite hue

    const cx = size / 2;
    const cy = size / 2;
    const R = size * 0.34;
    const f = 3.2; // perspective

    const cosY = Math.cos(this.rotY);
    const sinY = Math.sin(this.rotY);
    const cosT = Math.cos(this.tilt + Math.sin(t * 0.13) * 0.06);
    const sinT = Math.sin(this.tilt + Math.sin(t * 0.13) * 0.06);

    const project = (p: P3) => {
      // rotate Y then X (tilt)
      let x = p.x * cosY + p.z * sinY;
      let z = -p.x * sinY + p.z * cosY;
      let y = p.y * cosT - z * sinT;
      z = p.y * sinT + z * cosT;
      const s = f / (f + z);
      return { sx: cx + x * R * s, sy: cy + y * R * s, z, s };
    };

    // core glow
    const g = ctx.createRadialGradient(cx, cy, 0, cx, cy, R * 1.25);
    g.addColorStop(0, `hsla(${hue},100%,60%,${0.05 + breathe * 0.05})`);
    g.addColorStop(1, "transparent");
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, size, size);

    // points
    const projected = this.pts.map(project);
    for (const q of projected) {
      const depth = (q.z + 1) / 2; // 0 far → 1 near... z positive = away; invert
      const near = 1 - depth;
      const alphaBase = isWire ? 0.08 : 0.12;
      const alphaRange = isWire ? 0.3 : 0.5;
      const alpha = alphaBase + near * (alphaRange + breathe * 0.35);
      const rad = isWire ? 0.5 + near * 1.0 : 0.7 + near * 1.5;
      ctx.beginPath();
      ctx.arc(q.sx, q.sy, rad, 0, Math.PI * 2);
      ctx.fillStyle = `hsla(${hue},100%,${58 + near * 18}%,${alpha})`;
      ctx.fill();
    }

    // random node pings (more frequent under load)
    if (t - this.lastPing > 1.6 - load * 1.1) {
      this.lastPing = t;
      this.pings.push({ idx: Math.floor(Math.random() * this.pts.length), t: 0 });
    }
    // vault capture → burst of node pings
    if (activity.vaultAt && activity.vaultAt !== this.lastVaultSeen) {
      this.lastVaultSeen = activity.vaultAt;
      for (let i = 0; i < 7; i++) {
        this.pings.push({ idx: Math.floor(Math.random() * this.pts.length), t: 0 });
      }
    }
    this.pings = this.pings.filter((p) => p.t < 1);
    for (const p of this.pings) {
      p.t += 0.022;
      const q = projected[p.idx];
      const a = (1 - p.t) * 0.7;
      ctx.beginPath();
      ctx.arc(q.sx, q.sy, 2 + p.t * 16 * q.s, 0, Math.PI * 2);
      ctx.strokeStyle = `hsla(${hue},100%,70%,${a})`;
      ctx.lineWidth = 1.1;
      ctx.stroke();
    }

    // satellites — ping harder while an AI call is in flight
    const aiActive = activity.ai > 0;
    for (const s of this.sats) {
      s.angle += s.speed * 0.016 * (1 + load * 0.8);
      s.pingT += aiActive ? 0.035 : 0.012;
      if (s.pingT > 1) s.pingT = 0;
      const sp: P3 = {
        x: Math.cos(s.angle) * Math.cos(s.incl) * 1.45,
        y: Math.sin(s.incl) * 1.45,
        z: Math.sin(s.angle) * Math.cos(s.incl) * 1.45,
      };
      const q = project(sp);

      // connection line to nearest visible sphere node
      let best = -1;
      let bd = 1e9;
      for (let i = 0; i < projected.length; i += 7) {
        const n = projected[i];
        if (n.z > 0.2) continue; // prefer near-side nodes
        const d = (n.sx - q.sx) ** 2 + (n.sy - q.sy) ** 2;
        if (d < bd) {
          bd = d;
          best = i;
        }
      }
      if (best >= 0) {
        const n = projected[best];
        ctx.beginPath();
        ctx.moveTo(q.sx, q.sy);
        ctx.lineTo(n.sx, n.sy);
        ctx.strokeStyle = `hsla(${satHue},100%,62%,${0.3 * breathe})`;
        ctx.lineWidth = 0.8;
        ctx.stroke();
      }

      // satellite body + ping ripple (halo drawn cheaply, no shadowBlur)
      ctx.beginPath();
      ctx.arc(q.sx, q.sy, 5 * q.s, 0, Math.PI * 2);
      ctx.fillStyle = `hsla(${satHue},100%,65%,0.25)`;
      ctx.fill();
      ctx.beginPath();
      ctx.arc(q.sx, q.sy, 2.4 * q.s, 0, Math.PI * 2);
      ctx.fillStyle = `hsla(${satHue},100%,65%,0.95)`;
      ctx.fill();

      const pa = 1 - s.pingT;
      ctx.beginPath();
      ctx.arc(q.sx, q.sy, 3 + s.pingT * 14, 0, Math.PI * 2);
      ctx.strokeStyle = `hsla(${satHue},100%,65%,${pa * 0.5})`;
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    // equator ring
    ctx.beginPath();
    for (let a = 0; a <= Math.PI * 2 + 0.01; a += 0.08) {
      const q = project({ x: Math.cos(a) * 1.02, y: 0, z: Math.sin(a) * 1.02 });
      a === 0 ? ctx.moveTo(q.sx, q.sy) : ctx.lineTo(q.sx, q.sy);
    }
    ctx.strokeStyle = `hsla(${hue},100%,60%,${0.1 + breathe * 0.08})`;
    ctx.lineWidth = 0.7;
    ctx.stroke();

    // AI uplink ring — pulses magenta while a model call is in flight
    if (aiActive) {
      const pulse = 0.5 + 0.5 * Math.sin(t * 6);
      ctx.beginPath();
      ctx.arc(cx, cy, R * (1.14 + pulse * 0.05), 0, Math.PI * 2);
      ctx.strokeStyle = `hsla(${satHue},100%,65%,${0.2 + pulse * 0.3})`;
      ctx.lineWidth = 1.2;
      ctx.stroke();
    }
  }
}
