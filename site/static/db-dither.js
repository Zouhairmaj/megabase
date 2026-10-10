// Megabase hero illustration: animated ordered-dither database.
// Animates every <canvas data-db-dither> (150×150 native, upscaled with
// image-rendering: pixelated). Respects prefers-reduced-motion.
(() => {
  'use strict';

  const W = 150, H = 150;
  const TAU = Math.PI * 2, LOOP = 12.8, SPIN = TAU / 3.2; // every period divides LOOP
  const GREEN = [0, 216, 146], GREY = [48, 50, 53], DARK = [11, 14, 18];
  const cx = W / 2, rx = 54, ry = 15, h = 26, tops = [24, 64, 104];

  // 8x8 Bayer matrix, normalised to [0, 1)
  const B = [0,32,8,40,2,34,10,42,48,16,56,24,50,18,58,26,12,44,4,36,14,46,6,38,60,28,52,20,62,30,54,22,
             3,35,11,43,1,33,9,41,51,19,59,27,49,17,57,25,15,47,7,39,13,45,5,37,63,31,55,23,61,29,53,21].map(v => v / 64);

  // LED level: [disk][slot] in [0, 1]. Every 3.2 s another judge case "passes".
  // The last 3.2 s fades those extra greens back to the initial pattern so
  // the frame at t % LOOP == 0 matches the frame just before the wrap.
  const ORDER = [[1, 1], [2, 0], [2, 1]];
  function ledsAt(t) {
    const leds = [[1, 1], [1, 0], [0, 0]];
    const phase = (t % LOOP) / 3.2;
    if (phase < 3) {
      const flips = Math.floor(phase);
      for (let n = 0; n < flips; n++) leds[ORDER[n][0]][ORDER[n][1]] = 1;
    } else {
      const fade = 1 - (phase - 3);
      for (let n = 0; n < ORDER.length; n++) leds[ORDER[n][0]][ORDER[n][1]] = fade;
    }
    return leds;
  }

  function ledInk(level) {
    if (level >= 1) return GREEN;
    if (level <= 0) return GREY;
    return [
      Math.round(GREY[0] + (GREEN[0] - GREY[0]) * level),
      Math.round(GREY[1] + (GREEN[1] - GREY[1]) * level),
      Math.round(GREY[2] + (GREEN[2] - GREY[2]) * level),
    ];
  }

  function shadeAt(x, y, t) {
    const dx = (x - cx) / rx;
    if (Math.abs(dx) > 1) return -1;
    const curve = ry * Math.sqrt(Math.max(0, 1 - dx * dx));
    const bob = Math.sin(t * TAU / 6.4) * 1.5;          // gentle float
    const lightAng = 0.18 * Math.sin(t * TAU / LOOP);   // light drifts left/right
    let s = -1, hit = false;
    for (let i = 2; i >= 0; i--) {                      // bottom first, upper disks drawn over
      const top = tops[i] + bob;
      const inTop = dx * dx + ((y - top) / ry) ** 2 <= 1;
      const inBody = y >= top && y <= top + h + curve;
      if (!inTop && !inBody) continue;
      hit = true;
      const lit = Math.min(1, Math.max(0, 1 - (dx + 1) / 2 + lightAng));
      const dir = i % 2 ? -1 : 1;
      if (inTop) {
        // spinning top face: rotating rings
        const ang = Math.atan2((y - top) / ry, dx);
        s = 0.38 + 0.45 * lit - 0.05 * i + 0.06 * Math.sin(ang * 6 - t * SPIN * dir);
      } else {
        // cylinder side: ridges sliding around the disk = rotation
        const theta = Math.asin(Math.max(-1, Math.min(1, dx)));
        s = 0.05 + 0.68 * Math.pow(lit, 1.3) - 0.06 * i + 0.07 * Math.sin(theta * 10 - t * SPIN * dir);
        if (Math.abs(y - (top + h + curve)) < 1) s = 0.05; // rim shadow
      }
    }
    if (!hit) return -1;
    s = Math.max(0, s);
    // Data pulse every 3.2 s, measured along the cylinder surface (minus the
    // ellipse curve) so the band bends with the disks instead of a straight line.
    const pulseY = ((t % 3.2) / 3.2) * (H + 60) - 30;
    const d = (y - curve) - pulseY;
    return s + 0.2 * Math.exp(-(d * d) / 150);
  }

  function setup(cv) {
    const ctx = cv.getContext('2d');
    if (!ctx) return;
    const img = ctx.createImageData(W, H);
    const px = img.data;

    function fill(x0, y0, w, hh, c) {
      for (let y = y0; y < y0 + hh; y++) for (let x = x0; x < x0 + w; x++) {
        if (x < 0 || y < 0 || x >= W || y >= H) continue;
        const o = (y * W + x) * 4;
        px[o] = c[0]; px[o + 1] = c[1]; px[o + 2] = c[2]; px[o + 3] = 255;
      }
    }

    function draw(t) {
      for (let y = 0; y < H; y++) {
        for (let x = 0; x < W; x++) {
          const s = shadeAt(x, y, t);
          const o = (y * W + x) * 4;
          if (s < 0) { px[o + 3] = 0; continue; }
          const th = B[(y & 7) * 8 + (x & 7)];
          const c = s > th ? GREEN : (s + 0.45 > th ? GREY : null);
          if (!c) { px[o + 3] = 0; continue; }
          px[o] = c[0]; px[o + 1] = c[1]; px[o + 2] = c[2]; px[o + 3] = 255;
        }
      }
      // status LEDs
      const bob = Math.sin(t * TAU / 6.4) * 1.5;
      const leds = ledsAt(t);
      for (let i = 0; i < 3; i++) {
        const y = Math.round(tops[i] + h / 2 + 9 + bob);
        [cx + 22, cx + 32].forEach((x, k) => {
          const level = leds[i][k];
          const blink = level >= 1 && Math.sin(t * SPIN * 3 + i * 2 + k) > 0.92; // tiny activity flicker
          fill(Math.round(x) - 2, y - 2, 5, 5, DARK);
          fill(Math.round(x) - 1, y - 1, 3, 3, blink ? GREY : ledInk(level));
        });
      }
      ctx.putImageData(img, 0, 0);
    }

    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
    let visible = true, raf = 0, last = 0;
    const t0 = performance.now();

    function frame(now) {
      raf = 0;
      if (!visible || document.hidden || reduce.matches) return;
      if (now - last > 33) { draw((now - t0) / 1000); last = now; } // ~30 fps
      raf = requestAnimationFrame(frame);
    }
    function start() { if (!raf && visible && !document.hidden && !reduce.matches) raf = requestAnimationFrame(frame); }

    draw(4.0); // static frame (also the reduced-motion frame)
    if ('IntersectionObserver' in window) {
      new IntersectionObserver(([e]) => { visible = e.isIntersecting; start(); }).observe(cv);
    }
    document.addEventListener('visibilitychange', start);
    if (reduce.addEventListener) reduce.addEventListener('change', () => { if (reduce.matches) draw(4.0); start(); });
    start();
  }

  function init() { document.querySelectorAll('canvas[data-db-dither]').forEach(setup); }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
})();
