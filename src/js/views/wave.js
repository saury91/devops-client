// Breathing halo + heartbeat ring — a single calm rhythm around the ball.
// 设计原则：只保留两个层，且共用同一种"呼吸"语言 ——
//   1. 呼吸光晕：一团柔光按 4.2s 周期涨落，与 CSS 的 ball-breathe 同频，
//      球缘辉光（CSS）与外围光晕（canvas）一起吸一起吐；
//   2. 心跳环：心跳包到达时一圈细环从球缘扩散，事件反馈，平时不出现。
// 之前的雷达扫描、日冕射线、随机涟漪、粒子等全部移除 —— 律动来自节律，不来自数量。
var Wave = (function () {
  'use strict';

  var canvas, ctx, w, h;
  var animId;
  var running = false;
  var amplitude = 1.0;
  var targetAmp = 0.6;
  var shockwaves = [];          // 心跳扩散环
  var ballR = 75;               // 与 CSS 的 .monitor-ball 半径保持一致
  var BREATH_MS = 4200;         // 与 CSS ball-breathe 同周期
  var curTone = '0,229,255';    // 当前状态色（RGB 三元组），心跳失败转琥珀

  function emitLog(type, ok) {
    var event = new CustomEvent('wave-log', { detail: { type: type, ok: ok } });
    document.dispatchEvent(event);
  }

  function init() {
    if (running) return;                 // 重复 init 会挂上重复的 resize 监听并起两条 rAF 循环
    canvas = document.getElementById('wave-canvas');
    if (!canvas) return;
    ctx = canvas.getContext('2d');
    if (!ctx) return;
    running = true;
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
      shockwaves = [];
      if (running && !animId) loop();
    }
  }

  // 画布按 devicePixelRatio 放大后再用 CSS 像素坐标绘制：
  // Windows 缩放 125%/150% 时 dpr 不为 1，不处理的话整层光晕都是糊的
  function resize() {
    if (!canvas || !canvas.parentElement) return;
    var dpr = Math.min(window.devicePixelRatio || 1, 3);
    var rect = canvas.parentElement.getBoundingClientRect();
    w = Math.max(1, Math.round(rect.width));
    h = Math.max(1, Math.round(rect.height));
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    if (ctx.setTransform) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  // 心跳到达：一圈细亮环从球缘扩散消散
  function spawnShock() {
    if (shockwaves.length > 3) shockwaves.shift();
    shockwaves.push({ r: ballR + 2, alpha: 0.32, speed: 2.2, width: 1.5 });
  }

  function loop() {
    if (!ctx || !running) return;
    if (document.hidden) { animId = null; return; }
    ctx.clearRect(0, 0, w, h);

    var cx = w / 2;
    var cy = h / 2;

    // 平滑回归基线亮度（心跳时 targetAmp 抬高，随后缓缓落回）
    var dAmp = targetAmp - amplitude;
    amplitude += dAmp * 0.04;

    // 呼吸曲线：0..1..0，与 CSS ball-breathe 同一个 4.2s 节拍
    var phase = (Date.now() % BREATH_MS) / BREATH_MS;
    var breath = 0.5 - 0.5 * Math.cos(phase * Math.PI * 2);

    // --- 1. 呼吸光晕：一整团包裹球体的柔光，随节律一起吸吐 ---
    var glowR = ballR + 22 + breath * 14;
    var gAlpha = (0.035 + breath * 0.075) * amplitude;
    var grad = ctx.createRadialGradient(cx, cy, ballR - 6, cx, cy, glowR);
    grad.addColorStop(0, 'rgba(' + curTone + ',0)');
    grad.addColorStop(0.42, 'rgba(' + curTone + ',' + (gAlpha * 0.55).toFixed(4) + ')');
    grad.addColorStop(1, 'rgba(' + curTone + ',0)');
    ctx.beginPath();
    ctx.arc(cx, cy, glowR, 0, Math.PI * 2);
    ctx.fillStyle = grad;
    ctx.fill();

    // --- 2. 心跳扩散环（事件驱动，平时不可见） ---
    for (var si = shockwaves.length - 1; si >= 0; si--) {
      var sw = shockwaves[si];
      sw.r += sw.speed;
      sw.speed *= 0.985;
      sw.alpha *= 0.955;
      if (sw.alpha < 0.01) { shockwaves.splice(si, 1); continue; }
      ctx.beginPath();
      ctx.arc(cx, cy, sw.r, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(' + curTone + ',' + sw.alpha.toFixed(4) + ')';
      ctx.lineWidth = sw.width;
      ctx.stroke();
    }

    animId = requestAnimationFrame(loop);
  }

  function heartbeatOk() {
    curTone = '0,229,255';
    targetAmp = 1.8;
    setTimeout(function () { targetAmp = 0.6; }, 1500);
    if (canvas && !document.hidden) {
      spawnShock();
      // 球面脉冲：与 canvas 冲击波同帧出发，动画结束后自然停在透明态（forwards）
      var ball = document.getElementById('monitor-ball');
      if (ball) {
        ball.classList.remove('hb-pulse');
        void ball.offsetWidth;   // 强制重排，连续心跳也能重启动画
        ball.classList.add('hb-pulse');
      }
    }
    emitLog('HB', true);
  }

  function heartbeatFail() {
    curTone = '255,194,75';
    targetAmp = 0.25;
    emitLog('HB', false);
  }

  function stop() {
    running = false;
    if (animId) cancelAnimationFrame(animId);
    animId = null;
    shockwaves = [];
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
