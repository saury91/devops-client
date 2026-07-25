// Cyberpunk animated background — particle swarm + circuit traces + pulse rings.
var Background = (function () {
  'use strict';

  var canvas, ctx, w, h;
  var particles = [];
  var circuits = [];
  var pulses = [];
  var animId;
  var PARTICLE_COUNT = 60;

  function init() {
    canvas = document.getElementById('bg-canvas');
    if (!canvas) return;
    ctx = canvas.getContext('2d');
    resize();
    window.addEventListener('resize', resize);

    spawnParticles();
    spawnCircuits();
    loop();
  }

  function resize() {
    w = canvas.width = window.innerWidth;
    h = canvas.height = window.innerHeight;
  }

  // ---- Particles ----
  function spawnParticles() {
    for (var i = 0; i < PARTICLE_COUNT; i++) {
      particles.push({
        x: Math.random() * w,
        y: Math.random() * h,
        vx: (Math.random() - 0.5) * 0.4,
        vy: (Math.random() - 0.5) * 0.4,
        r: Math.random() * 1.5 + 0.5,
        alpha: Math.random() * 0.5 + 0.15
      });
    }
  }

  function updateParticles() {
    for (var i = 0; i < particles.length; i++) {
      var p = particles[i];
      p.x += p.vx;
      p.y += p.vy;

      if (p.x < 0) p.x = w;
      if (p.x > w) p.x = 0;
      if (p.y < 0) p.y = h;
      if (p.y > h) p.y = 0;
    }
  }

  function drawParticles() {
    // Draw connections
    for (var i = 0; i < particles.length; i++) {
      for (var j = i + 1; j < particles.length; j++) {
        var dx = particles[i].x - particles[j].x;
        var dy = particles[i].y - particles[j].y;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 100) {
          ctx.beginPath();
          ctx.moveTo(particles[i].x, particles[i].y);
          ctx.lineTo(particles[j].x, particles[j].y);
          ctx.strokeStyle = 'rgba(0,229,255,' + (0.04 * (1 - dist / 100)) + ')';
          ctx.lineWidth = 0.5;
          ctx.stroke();
        }
      }
    }

    // Draw dots
    for (var k = 0; k < particles.length; k++) {
      var p = particles[k];
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(0,229,255,' + p.alpha + ')';
      ctx.fill();
    }
  }

  // ---- Circuit Traces ----
  function spawnCircuits() {
    circuits.push({ segments: makeCircuit(), speed: 0.3, offset: 0 });
    circuits.push({ segments: makeCircuit(), speed: 0.5, offset: 0.3 });
  }

  function makeCircuit() {
    var segs = [];
    var cx = Math.random() * w * 0.8 + w * 0.1;
    var cy = Math.random() * h * 0.8 + h * 0.1;
    for (var s = 0; s < 5; s++) {
      var len = 30 + Math.random() * 60;
      var dir = Math.floor(Math.random() * 4);
      var nx = cx, ny = cy;
      if (dir === 0) nx += len;
      if (dir === 1) ny += len;
      if (dir === 2) nx -= len;
      if (dir === 3) ny -= len;
      segs.push({ x1: cx, y1: cy, x2: nx, y2: ny });
      cx = nx; cy = ny;
    }
    return segs;
  }

  function drawCircuits() {
    var t = Date.now() / 1000;
    for (var c = 0; c < circuits.length; c++) {
      var circ = circuits[c];
      var progress = (t * circ.speed + circ.offset) % 2;
      if (progress > 1) progress = 2 - progress;

      ctx.strokeStyle = 'rgba(0,229,255,' + (0.06 + progress * 0.04) + ')';
      ctx.lineWidth = 1;
      for (var s = 0; s < circ.segments.length; s++) {
        var seg = circ.segments[s];
        ctx.beginPath();
        ctx.moveTo(seg.x1, seg.y1);
        ctx.lineTo(seg.x2, seg.y2);
        ctx.stroke();
      }
    }
  }

  // ---- Pulse Rings ----
  function drawPulses() {
    var t = Date.now() / 1000;
    // Spawn new pulses occasionally
    if (Math.random() < 0.02 && pulses.length < 4) {
      pulses.push({
        x: Math.random() * w,
        y: Math.random() * h,
        start: t,
        life: 2.5 + Math.random() * 2
      });
    }

    for (var i = pulses.length - 1; i >= 0; i--) {
      var pulse = pulses[i];
      var age = t - pulse.start;
      if (age > pulse.life) { pulses.splice(i, 1); continue; }
      var progress = age / pulse.life;
      var radius = progress * 120;
      var alpha = (1 - progress) * 0.1;
      ctx.beginPath();
      ctx.arc(pulse.x, pulse.y, radius, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(0,229,255,' + alpha + ')';
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }

  // ---- Main Loop ----
  function loop() {
    ctx.clearRect(0, 0, w, h);

    // Grid lines (static, subtle)
    ctx.strokeStyle = 'rgba(0,229,255,0.018)';
    ctx.lineWidth = 0.5;
    var gridSize = 40;
    for (var gx = gridSize; gx < w; gx += gridSize) {
      ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, h); ctx.stroke();
    }
    for (var gy = gridSize; gy < h; gy += gridSize) {
      ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(w, gy); ctx.stroke();
    }

    drawCircuits();
    updateParticles();
    drawParticles();
    drawPulses();

    animId = requestAnimationFrame(loop);
  }

  function stop() {
    if (animId) cancelAnimationFrame(animId);
  }

  return { init: init, stop: stop };
})();
