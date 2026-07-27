// Corona glow + ripple burst monitor — organic animation around the center ball.
var Wave = (function () {
  'use strict';

  var canvas, ctx, w, h;
  var animId;
  var amplitude = 1.0;          // baseline glow intensity
  var targetAmp = 0.6;
  var glowPhase = 0;            // rotation phase
  var ripples = [];
  var spawnTimer = 0;
  var ballR = 75;
  var CORONA_RAYS = 18;
  var coronaJitter = [];        // per-ray irregular offsets

  function emitLog(type, ok) {
    var event = new CustomEvent('wave-log', { detail: { type: type, ok: ok } });
    document.dispatchEvent(event);
  }

  function init() {
    canvas = document.getElementById('wave-canvas');
    if (!canvas) return;
    ctx = canvas.getContext('2d');
    // Init per-ray random jitter values
    coronaJitter = [];
    for (var j = 0; j < CORONA_RAYS; j++) {
      coronaJitter.push({
        speed: 0.5 + Math.random() * 2.5,
        phase: Math.random() * Math.PI * 2,
        lenVar: 0.3 + Math.random() * 0.7
      });
    }
    resize();
    window.addEventListener('resize', resize);
    document.addEventListener('visibilitychange', onVisibilityChange);
    loop();
  }

  function onVisibilityChange() {
    if (document.hidden) {
      if (animId) cancelAnimationFrame(animId);
      animId = null;
    } else {
      ripples = [];
      spawnTimer = 0;
      if (!animId) loop();
    }
  }

  function resize() {
    if (!canvas || !canvas.parentElement) return;
    var rect = canvas.parentElement.getBoundingClientRect();
    w = canvas.width = Math.max(1, rect.width);
    h = canvas.height = Math.max(1, rect.height);
  }

  function spawnRipple(cx, cy, count) {
    count = count || 1;
    var maxTotal = 60;
    if (ripples.length + count > maxTotal) {
      ripples.splice(0, ripples.length + count - maxTotal);
    }
    for (var i = 0; i < count; i++) {
      var angle = Math.random() * Math.PI * 2;
      var edgeX = cx + Math.cos(angle) * ballR;
      var edgeY = cy + Math.sin(angle) * ballR;
      var outward = 5 + Math.random() * 25;
      ripples.push({
        x: edgeX + Math.cos(angle) * outward,
        y: edgeY + Math.sin(angle) * outward,
        radius: 4 + Math.random() * 12,
        speed: 0.4 + Math.random() * 1.6,
        maxAge: 70 + Math.random() * 90,
        age: 0,
        baseAlpha: 0.08 + Math.random() * 0.22,
        lineWidth: 0.6 + Math.random() * 1.4
      });
    }
  }

  function loop() {
    if (!ctx) return;
    if (document.hidden) { animId = null; return; }
    ctx.clearRect(0, 0, w, h);

    // Smooth amplitude transition with random walk
    var dAmp = targetAmp - amplitude;
    amplitude += dAmp * (0.03 + Math.random() * 0.02);

    glowPhase += 0.003;
    var cx = w / 2;
    var cy = h / 2;
    var t = Date.now() / 1000; // seconds for noise

    // --- 1. Corona glow rings (subtle, tight around ball) ---
    for (var i = 2; i >= 0; i--) {
      var ringStart = ballR + 2 + i * 5;
      var ringEnd = ringStart + 8;
      var noise = Math.sin(t * 1.7 + i * 2.1) * 0.06 + Math.sin(t * 3.3 + i) * 0.04 + (Math.random() - 0.5) * 0.02;
      var breathe = 1 + noise;
      var ringAlpha = (0.008 + i * 0.004) * amplitude * (0.85 + Math.random() * 0.3);
      var grad = ctx.createRadialGradient(cx, cy, ringStart * breathe, cx, cy, ringEnd * breathe);
      grad.addColorStop(0, 'rgba(0,229,255,' + ringAlpha + ')');
      grad.addColorStop(0.5, 'rgba(0,229,255,' + (ringAlpha * 0.4) + ')');
      grad.addColorStop(1, 'rgba(0,229,255,0)');
      ctx.beginPath();
      ctx.arc(cx, cy, ringEnd * breathe, 0, Math.PI * 2);
      ctx.fillStyle = grad;
      ctx.fill();
    }

    // --- 2. Inner bright rim (very faint) ---
    var rimAlpha = (0.02 + Math.random() * 0.01) * amplitude;
    var rimGrad = ctx.createRadialGradient(cx, cy, ballR - 1, cx, cy, ballR + 5);
    rimGrad.addColorStop(0, 'rgba(0,229,255,0)');
    rimGrad.addColorStop(0.5, 'rgba(0,229,255,' + rimAlpha + ')');
    rimGrad.addColorStop(1, 'rgba(0,229,255,0)');
    ctx.beginPath();
    ctx.arc(cx, cy, ballR + 5, 0, Math.PI * 2);
    ctx.fillStyle = rimGrad;
    ctx.fill();

    // --- 3. Corona rays (short, subtle) ---
    for (var j = 0; j < CORONA_RAYS; j++) {
      var jit = coronaJitter[j];
      var rayAngle = (j / CORONA_RAYS) * Math.PI * 2 + glowPhase * (0.5 + jit.speed * 0.3);
      var rayLen = 6 + Math.sin(t * jit.speed + jit.phase) * 5 * jit.lenVar + (Math.random() - 0.5) * 2;
      var rayAlpha = (0.006 + 0.008 * Math.sin(t * 2.1 + j) + (Math.random() - 0.5) * 0.004) * amplitude;
      var rayStart = ballR + 1;
      var rayEnd = ballR + Math.max(2, rayLen);

      var sx = cx + Math.cos(rayAngle) * rayStart;
      var sy = cy + Math.sin(rayAngle) * rayStart;
      var ex = cx + Math.cos(rayAngle) * rayEnd;
      var ey = cy + Math.sin(rayAngle) * rayEnd;

      ctx.beginPath();
      ctx.moveTo(sx, sy);
      ctx.lineTo(ex, ey);
      ctx.strokeStyle = 'rgba(0,229,255,' + rayAlpha + ')';
      ctx.lineWidth = 0.5 + Math.random() * 0.6;
      ctx.stroke();
    }

    // --- 4. Floating particles (fewer, fainter) ---
    var particleCount = 8;
    for (var k = 0; k < particleCount; k++) {
      var pAngle = glowPhase * 0.6 + (k / particleCount) * Math.PI * 2;
      var pDist = ballR + 5 + Math.sin(t * (1.5 + k * 0.3)) * (3 + Math.random() * 2);
      var px = cx + Math.cos(pAngle) * pDist;
      var py = cy + Math.sin(pAngle) * pDist;
      var pAlpha = (0.02 + Math.random() * 0.02) * amplitude;

      ctx.beginPath();
      ctx.arc(px, py, 0.5 + Math.random() * 0.5, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(0,229,255,' + pAlpha + ')';
      ctx.fill();
    }

    // --- 5. Ripple bursts (original animation restored) ---
    spawnTimer++;
    var spawnRate = Math.max(6, 28 - amplitude * 8);
    if (spawnTimer > spawnRate && Math.random() > 0.25) {
      spawnRipple(cx, cy, 1 + Math.floor(Math.random() * 2));
      spawnTimer = 0;
    }

    for (var ri = ripples.length - 1; ri >= 0; ri--) {
      var r = ripples[ri];
      r.radius += r.speed;
      r.age++;
      var life = r.age / r.maxAge;
      if (life >= 1) { ripples.splice(ri, 1); continue; }
      var alpha = r.baseAlpha * (1 - life) * amplitude;
      if (alpha <= 0.005) continue;
      ctx.beginPath();
      ctx.arc(r.x, r.y, r.radius, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(0,229,255,' + alpha + ')';
      ctx.lineWidth = r.lineWidth * (1 - life * 0.5);
      ctx.stroke();
    }

    animId = requestAnimationFrame(loop);
  }

  function heartbeatOk() {
    targetAmp = 2.0;
    setTimeout(function () { targetAmp = 0.6; }, 1500);
    if (canvas && !document.hidden) {
      var rect = canvas.getBoundingClientRect();
      spawnRipple(rect.width / 2, rect.height / 2, 3);
    }
    emitLog('HB', true);
  }

  function heartbeatFail() {
    targetAmp = 0.25;
    emitLog('HB', false);
  }

  function stop() {
    if (animId) cancelAnimationFrame(animId);
    animId = null;
    ripples = [];
    window.removeEventListener('resize', resize);
    document.removeEventListener('visibilitychange', onVisibilityChange);
  }

  return { init: init, resize: resize, heartbeatOk: heartbeatOk, heartbeatFail: heartbeatFail, stop: stop };
})();

document.addEventListener('wave-log', function (e) {
  if (typeof Panel !== 'undefined' && Panel.addLog) {
    Panel.addLog(e.detail.type, e.detail.ok);
  }
});
