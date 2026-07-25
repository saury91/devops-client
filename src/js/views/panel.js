// Panel view — user info, tabs (wave + log), request log.
var Panel = (function () {
  'use strict';

  var _state;
  var _startTime;
  var _timerId;
  var _logs = [];
  var _ballClickHandler;
  var _openingBrowser = false;
  var MAX_LOGS = 1000;

  function init() {
    initTabs();
  }

  function initTabs() {
    var tabs = document.querySelectorAll('.panel-tab');
    tabs.forEach(function (tab) {
      tab.addEventListener('click', function () {
        var target = tab.getAttribute('data-tab');
        tabs.forEach(function (t) { t.classList.remove('active'); });
        tab.classList.add('active');
        document.querySelectorAll('.panel-tab-content').forEach(function (c) {
          c.classList.toggle('active', c.id === 'tab-' + target);
        });
        // Resize wave canvas when its tab becomes visible
        if (target === 'wave') Wave.resize();
      });
    });
  }

  async function show(state) {
    _state = state;
    _startTime = Date.now();

    // Initial fetch + re-fetch whenever the panel becomes visible
    await refreshUserInfo();
    document.addEventListener('visibilitychange', onVisibilityChange);

    // Monitor ball opens dashboard
    var ball = document.getElementById('monitor-ball');
    if (ball) {
      _ballClickHandler = function () { openDashboard(ball); };
      ball.addEventListener('click', _ballClickHandler);
    }

    // Start uptime timer
    _timerId = setInterval(updateUptime, 50);
    updateUptime();

    // Init wave
    Wave.init();

    // Initial log entries
    _logs = [];
    addLogEntry('AGENT', 'started');
    renderLogs();
  }

  async function refreshUserInfo() {
    if (!_state || !_state.serverUrl || !_state.token) return;
    try {
      var userInfo = await API.getUserInfo(_state.serverUrl, _state.token);
      updateUserInfo(userInfo);
      addLogEntry('USER', 'ok');
      renderLogs();
    } catch (e) {
      addLogEntry('USER', 'fail');
      renderLogs();
    }
  }

  function onVisibilityChange() {
    if (!document.hidden) {
      refreshUserInfo();
    }
  }

  function updateUserInfo(userInfo) {
    var nickname = (userInfo && userInfo.nickname) || _state.nickname || _state.username || '-';
    var username = (userInfo && userInfo.username) || _state.username || '-';
    var avatarUrl = userInfo && userInfo.avatar ? userInfo.avatar : '';

    document.getElementById('p-nick').textContent = nickname;
    document.getElementById('p-login-time').textContent = (_state && _state.loginAt) || '-';
    var platform = getPlatform();
    var osVersion = getOSVersion();
    document.getElementById('p-platform').textContent = platform + (osVersion !== '-' ? ' (' + osVersion + ')' : '');

    var avatarImg = document.getElementById('p-avatar');
    var avatarFallback = document.getElementById('p-avatar-fallback');
    if (avatarImg && avatarFallback) {
      var displayName = (nickname !== '-' ? nickname : username);
      var initial = displayName.charAt(0).toUpperCase();
      if (avatarUrl) {
        avatarImg.src = avatarUrl;
        avatarImg.style.display = 'block';
        avatarFallback.style.display = 'none';
      } else {
        avatarImg.src = AvatarUtil.generateDefaultAvatar(initial);
        avatarImg.style.display = 'block';
        avatarFallback.style.display = 'none';
      }
    }
  }

  function getPlatform() {
    if (window.navigator.platform) return window.navigator.platform;
    if (window.navigator.userAgentData && window.navigator.userAgentData.platform) {
      return window.navigator.userAgentData.platform;
    }
    return '-';
  }

  function getOSVersion() {
    var ua = window.navigator.userAgent || '';
    var match;
    if ((match = ua.match(/Mac OS X ([\d_]+)/))) return 'macOS ' + match[1].replace(/_/g, '.');
    if ((match = ua.match(/Windows NT ([\d.]+)/))) {
      var map = { '10.0': '10/11', '6.3': '8.1', '6.2': '8', '6.1': '7' };
      return 'Windows ' + (map[match[1]] || match[1]);
    }
    if ((match = ua.match(/Android ([\d.]+)/))) return 'Android ' + match[1];
    if ((match = ua.match(/(?:iPhone|iPad|iPod) OS ([\d_]+)/))) return 'iOS ' + match[1].replace(/_/g, '.');
    if (ua.indexOf('Linux') !== -1) return 'Linux';
    return '-';
  }

  function updateUptime() {
    var elapsed = Date.now() - _startTime;
    var ms = elapsed % 1000;
    var sec = Math.floor(elapsed / 1000) % 60;
    var min = Math.floor(elapsed / 60000) % 60;
    var hr = Math.floor(elapsed / 3600000);
    var mainEl = document.getElementById('p-uptime-main');
    var msEl = document.getElementById('p-uptime-ms');
    var ballUptime = document.querySelector('.ball-uptime');
    if (mainEl) {
      mainEl.textContent =
        String(hr).padStart(2, '0') + ':' +
        String(min).padStart(2, '0') + ':' +
        String(sec).padStart(2, '0');
    }
    if (msEl) {
      msEl.textContent = '.' + String(ms).padStart(3, '0');
    }
    if (ballUptime) {
      ballUptime.classList.remove('hours-2', 'hours-3');
      if (hr >= 100) {
        ballUptime.classList.add('hours-3');
      } else if (hr >= 10) {
        ballUptime.classList.add('hours-2');
      }
    }
  }

  function heartbeatResult(ok) {
    if (ok) Wave.heartbeatOk();
    else Wave.heartbeatFail();
  }

  function addLog(type, ok) {
    addLogEntry(type, ok ? 'ok' : 'fail');
    renderLogs();
  }

  function addLogEntry(type, status) {
    var now = new Date();
    var ts = now.getHours().toString().padStart(2, '0') + ':' +
             now.getMinutes().toString().padStart(2, '0') + ':' +
             now.getSeconds().toString().padStart(2, '0') + '.' +
             now.getMilliseconds().toString().padStart(3, '0');
    _logs.unshift({ time: ts, type: type, status: status });
    if (_logs.length > MAX_LOGS) _logs.pop();
  }

  function renderLogs() {
    var el = document.getElementById('log-list');
    if (!el) return;
    var html = '';
    for (var i = 0; i < _logs.length; i++) {
      var entry = _logs[i];
      var ok = entry.status === 'ok' || entry.status === 'started';
      var dot = ok ? '<span class="log-ok">&#10003;</span>'
                   : '<span class="log-fail">&#10007;</span>';
      html += '<div class="log-row">' +
        '<span class="log-time">' + entry.time + '</span>' +
        '<span class="log-type">' + entry.type + '</span>' +
        dot +
        '</div>';
    }
    el.innerHTML = html;
  }

  function clearLogs() {
    _logs = [];
    renderLogs();
  }

  function cleanup() {
    if (_timerId) clearInterval(_timerId);
    document.removeEventListener('visibilitychange', onVisibilityChange);
    var ball = document.getElementById('monitor-ball');
    if (ball && _ballClickHandler) ball.removeEventListener('click', _ballClickHandler);
    _ballClickHandler = null;
    _openingBrowser = false;
    Wave.stop();
  }

  async function openDashboard(ball) {
    if (!_state || _openingBrowser) return;
    _openingBrowser = true;
    if (ball) ball.classList.add('opening');

    try {
      if (_state.token) {
        try {
          await API.openDashboard(_state.serverUrl, _state.token, _state.port || 0);
          return;
        } catch (_) {}
      }
      await API.openBrowser(_state.serverUrl);
    } finally {
      _openingBrowser = false;
      if (ball) ball.classList.remove('opening');
    }
  }

  // Wire clear log button from init
  document.addEventListener('DOMContentLoaded', function () {
    var clearBtn = document.getElementById('clear-log-btn');
    if (clearBtn) clearBtn.addEventListener('click', clearLogs);
  });

  return {
    init: init, show: show, cleanup: cleanup,
    heartbeatResult: heartbeatResult,
    addLog: addLog
  };
})();
