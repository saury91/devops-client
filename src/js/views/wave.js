// Radial surge wave monitor — organic ripples bursting outward from the center ball.
var Wave = (function () {
  'use strict';

  var canvas, ctx, w, h;
  var animId;
  var amplitude = 1.0;   // baseline activity multiplier
  var targetAmp = 0.6;
  var ripples = [];
  var spawnTimer = 0;
  var ballR = 75;        // radius of the center monitor ball

  function init() {
    canvas = document.getElementById('wave-canvas');
    if (!canvas) return;
    ctx = canvas.getContext('2d');
    resize();
    window.addEventListener('resize', resize);
    document.addEventListener('visibilitychange', onVisibilityChange);
    loop();
  }

  function onVisibilityChange() {
    if (document.hidden) {
      // Stop animation while hidden to avoid accumulating ripples.
      if (animId) cancelAnimationFrame(animId);
      animId = null;
    } else {
      // Clear stale ripples that accumulated during hidden state and restart loop.
      ripples = [];
      spawnTimer = 0;
      if (!animId) loop();
    }
  }

  function resize() {
    if (!canvas) return;
    var rect = canvas.parentElement.getBoundingClientRect();
    w = canvas.width = rect.width;
    h = canvas.height = rect.height;
  }

  function spawnRipple(cx, cy, count) {
    count = count || 1;
    // Cap total ripples to prevent visual clutter and memory growth.
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
    if (document.hidden) {
      animId = null;
      return;
    }
    ctx.clearRect(0, 0, w, h);

    // Smooth amplitude transition
    amplitude += (targetAmp - amplitude) * 0.05;

    var cx = w / 2;
    var cy = h / 2;

    // Auto-spawn ambient ripples
    spawnTimer++;
    var spawnRate = Math.max(6, 28 - amplitude * 8);
    if (spawnTimer > spawnRate && Math.random() > 0.25) {
      spawnRipple(cx, cy, 1 + Math.floor(Math.random() * 2));
      spawnTimer = 0;
    }

    // Update and draw ripples
    for (var i = ripples.length - 1; i >= 0; i--) {
      var r = ripples[i];
      r.radius += r.speed;
      r.age++;
      var life = r.age / r.maxAge;
      if (life >= 1) {
        ripples.splice(i, 1);
        continue;
      }

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

  // Call on activity (ping response / heartbeat success)
  function heartbeatOk() {
    targetAmp = 1.8;
    setTimeout(function () { targetAmp = 0.6; }, 1500);
    if (canvas && !document.hidden) {
      var rect = canvas.getBoundingClientRect();
      spawnRipple(rect.width / 2, rect.height / 2, 3);
    }
    Panel.addLog('HB', true);
  }

  function heartbeatFail() {
    targetAmp = 0.3;
    Panel.addLog('HB', false);
  }

  function stop() {
    if (animId) cancelAnimationFrame(animId);
    animId = null;
    ripples = [];
    document.removeEventListener('visibilitychange', onVisibilityChange);
  }

  return { init: init, resize: resize, heartbeatOk: heartbeatOk, heartbeatFail: heartbeatFail, stop: stop };
})();
