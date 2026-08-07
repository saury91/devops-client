// Panel view — user info, tabs (wave + log), device info modal, diagnostics modal.
var Panel = (function () {
  'use strict';

  var _state;
  var _startTime;
  var _timerId;
  var _logs = [];
  var _ballClickHandler;
  var _openingBrowser = false;
  var _lastHeartbeatTime = null;
  var _serverLatency = '-';
  var _lastEvent = '-';
  var _deviceInfo = null;
  var _avatarClicks = 0;
  var _avatarClickTimer = 0;
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
        if (target === 'wave') Wave.resize();
      });
    });
  }

  async function show(state) {
    _state = state;
    _startTime = Date.now();
    _lastHeartbeatTime = Date.now();

    // Init log before anything else so subsequent entries accumulate
    _logs = [];
    addLogEntry('AGENT', 'started');

    // Get hardware info from local system — cached for modal display
    try {
      _deviceInfo = await API.getDeviceInfo();
      document.getElementById('p-version').textContent = (_deviceInfo && _deviceInfo.clientVersion) || '-';
    } catch (e) {
      console.error('getDeviceInfo failed:', e);
      _deviceInfo = null;
    }

    // Fetch user info from server for avatar/nickname (adds USER entry)
    await refreshUserInfo();
    renderLogs();
    document.addEventListener('visibilitychange', onVisibilityChange);

    // Avatar triple-click → copy fingerprint
    wireAvatarCopy();

    var ball = document.getElementById('monitor-ball');
    if (ball) {
      _ballClickHandler = function () { openDashboard(ball); };
      ball.addEventListener('click', _ballClickHandler);
    }

    _timerId = setInterval(updateUptime, 50);
    updateUptime();

    // Init wave + heartbeat callbacks (adds HB entries as they fire)
    Wave.init();
  }

  function wireAvatarCopy() {
    _avatarClicks = 0;
    var img = document.getElementById('p-avatar');
    var fb = document.getElementById('p-avatar-fallback');
    var el = (img && img.style.display !== 'none') ? img : fb;
    if (!el || el._copyWired) return;
    el._copyWired = true;
    el.style.cursor = 'pointer';
    el.addEventListener('click', function () {
      _avatarClicks++;
      if (_avatarClicks === 1) {
        _avatarClickTimer = setTimeout(function () { _avatarClicks = 0; }, 800);
      } else if (_avatarClicks >= 3) {
        clearTimeout(_avatarClickTimer);
        _avatarClicks = 0;
        if (_state && _state.fingerprint) {
          navigator.clipboard.writeText(_state.fingerprint).then(function () {
            showToast(I18n.t('panel.fingerprintCopied'));
          }).catch(function () {
            showToast(_state.fingerprint.substring(0, 16) + '...');
          });
        }
      }
    });
  }

  function showToast(msg) {
    var el = document.createElement('div');
    el.style.cssText = 'position:fixed;top:50%;left:50%;transform:translate(-50%,-50%);z-index:9999;padding:6px 12px;border-radius:4px;background:rgba(0,229,255,0.12);border:1px solid var(--accent);color:var(--accent);font-size:11px;pointer-events:none;';
    el.textContent = msg;
    document.body.appendChild(el);
    setTimeout(function () { if (el.parentNode) el.parentNode.removeChild(el); }, 1500);
  }

  async function refreshUserInfo() {
    if (!_state || !_state.serverUrl || !_state.token) return;
    var start = Date.now();
    try {
      var userInfo = await API.getUserInfo(_state.serverUrl, _state.token);
      _serverLatency = (Date.now() - start) + 'ms';
      updateUserInfo(userInfo);
      addLogEntry('USER', 'ok');
      renderLogs();
    } catch (e) {
      _serverLatency = '-';
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

  // --- Device info modal ---
  function showDeviceInfoModal() {
    if (!_deviceInfo) return;
    var fields = [
      { key: 'hostname', label: 'Hostname' },
      { key: 'os',       label: I18n.t('panel.deviceOs') },
      { key: 'osVersion',label: 'OS Version' },
      { key: 'serial',   label: I18n.t('panel.deviceSerial') },
      { key: 'model',    label: I18n.t('panel.deviceModel') },
      { key: 'cpu',      label: I18n.t('panel.deviceCpu') },
      { key: 'gpu',      label: I18n.t('panel.deviceGpu') },
      { key: 'memory',   label: I18n.t('panel.deviceMemory') },
      { key: 'disk',     label: I18n.t('panel.deviceDisk') },
    ];
    var html = '';
    for (var i = 0; i < fields.length; i++) {
      var val = _deviceInfo[fields[i].key];
      if (val) {
        html += '<div class="di-row"><span class="di-label">' + fields[i].label + '</span><span>' + val + '</span></div>';
      }
    }
    var el = document.getElementById('device-info-modal-content');
    if (el) {
      el.innerHTML = html || '<div style="opacity:0.5">' + I18n.t('panel.noDeviceInfo') + '</div>';
    }
    document.getElementById('device-info-overlay').classList.add('active');
  }

  function hideDeviceInfoModal() {
    document.getElementById('device-info-overlay').classList.remove('active');
  }

  // --- Diagnostics modal ---
  async function showDiagModal() {
    // Refresh latency with a quick ping (keep last value on failure)
    if (_state && _state.serverUrl) {
      try {
        var r = await API.testConnection(_state.serverUrl);
        if (r && r.ok) _serverLatency = r.latency + 'ms';
      } catch (_) {}
    }

    var rows = [
      { label: I18n.t('panel.diagPort'),        value: (_state && _state.port) ? String(_state.port) : '-' },
      { label: I18n.t('panel.diagLatency'),     value: _serverLatency },
      { label: I18n.t('panel.diagLastHb'),      value: _lastHeartbeatTime ? formatTime(new Date(_lastHeartbeatTime)) : '-' },
      { label: I18n.t('panel.diagLastEvent'),   value: _lastEvent || '-' },
    ];
    var html = '';
    for (var i = 0; i < rows.length; i++) {
      html += '<div class="di-row"><span class="di-label">' + rows[i].label + '</span><span id="diag-val-' + i + '">' + rows[i].value + '</span></div>';
    }
    var el = document.getElementById('diag-modal-content');
    if (el) el.innerHTML = html;
    document.getElementById('diag-overlay').classList.add('active');

    // Start live refresh while modal is open
    startDiagRefresh();
  }

  var _diagRefreshId = 0;
  function startDiagRefresh() {
    if (_diagRefreshId) clearInterval(_diagRefreshId);
    _diagRefreshId = setInterval(function () {
      if (!document.getElementById('diag-overlay').classList.contains('active')) {
        clearInterval(_diagRefreshId);
        _diagRefreshId = 0;
        return;
      }
      if (_lastHeartbeatTime) {
        var hbEl = document.getElementById('diag-val-2');
        if (hbEl) hbEl.textContent = formatTime(new Date(_lastHeartbeatTime));
      }
      var evtEl = document.getElementById('diag-val-3');
      if (evtEl) evtEl.textContent = _lastEvent || '-';
      var latEl = document.getElementById('diag-val-1');
      if (latEl) latEl.textContent = _serverLatency;
    }, 1000);
  }

  function hideDiagModal() {
    document.getElementById('diag-overlay').classList.remove('active');
    if (_diagRefreshId) { clearInterval(_diagRefreshId); _diagRefreshId = 0; }
  }

  function getPlatform() {
    if (window.navigator.userAgentData && window.navigator.userAgentData.platform) {
      return window.navigator.userAgentData.platform;
    }
    if (window.navigator.platform) return window.navigator.platform;
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
      if (hr >= 100) ballUptime.classList.add('hours-3');
      else if (hr >= 10) ballUptime.classList.add('hours-2');
    }
  }

  function formatTime(d) {
    return d.getHours().toString().padStart(2, '0') + ':' +
           d.getMinutes().toString().padStart(2, '0') + ':' +
           d.getSeconds().toString().padStart(2, '0') + '.' +
           d.getMilliseconds().toString().padStart(3, '0');
  }

  function addLog(type, ok) {
    if (type === 'HB' || type === 'PING') _lastHeartbeatTime = Date.now();
    _lastEvent = type + ' ' + (ok ? 'ok' : 'fail');
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

  async function exportLogs() {
    var text = '';
    for (var i = 0; i < _logs.length; i++) {
      var e = _logs[i];
      text += e.time + ' [' + e.type + '] ' + e.status + '\n';
    }
    try {
      var path = await window.__TAURI__.dialog.save({
        defaultPath: 'agent-log-' + new Date().toISOString().slice(0, 10) + '.log',
        filters: [{ name: 'Log Files', extensions: ['log'] }]
      });
      if (!path) return; // User cancelled
      await API.exportLogFile(text, path);
      showToast(I18n.t('panel.logExported'));
    } catch (err) {
      console.error('exportLogs failed:', err);
    }
  }

  function cleanup() {
    if (_timerId) clearInterval(_timerId);
    document.removeEventListener('visibilitychange', onVisibilityChange);
    var ball = document.getElementById('monitor-ball');
    if (ball && _ballClickHandler) ball.removeEventListener('click', _ballClickHandler);
    _ballClickHandler = null;
    _openingBrowser = false;
    _deviceInfo = null;
    _avatarClicks = 0;
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

  // --- Change password modal ---
  function openChangePw() {
    document.getElementById('change-pw-old').value = '';
    document.getElementById('change-pw-new').value = '';
    document.getElementById('change-pw-confirm').value = '';
    var errEl = document.getElementById('change-pw-error');
    if (errEl) errEl.textContent = '';
    document.getElementById('change-pw-overlay').classList.add('active');
    var oldInput = document.getElementById('change-pw-old');
    if (oldInput) setTimeout(function () { oldInput.focus(); }, 60);
  }

  function hideChangePw() {
    document.getElementById('change-pw-overlay').classList.remove('active');
  }

  async function submitChangePw() {
    var oldPw = document.getElementById('change-pw-old').value;
    var newPw = document.getElementById('change-pw-new').value;
    var confirmPw = document.getElementById('change-pw-confirm').value;
    var errEl = document.getElementById('change-pw-error');
    if (!oldPw || !newPw) { if (errEl) errEl.textContent = I18n.t('login.fillAll'); return; }
    if (newPw.length < 10) { if (errEl) errEl.textContent = I18n.t('panel.pwTooShort'); return; }
    if (newPw !== confirmPw) { if (errEl) errEl.textContent = I18n.t('panel.pwMismatch'); return; }
    if (!_state || !_state.serverUrl) { if (errEl) errEl.textContent = I18n.t('panel.noServer'); return; }

    var btn = document.getElementById('change-pw-submit');
    btn.disabled = true;
    try {
      await API.changePassword(_state.serverUrl, _state.token, oldPw, newPw);
      hideChangePw();
      showToast(I18n.t('panel.pwChanged'));
      // 修改成功：自动退出，要求用新密码重新登录
      setTimeout(function () { if (window.App) App.logout(); }, 800);
    } catch (e) {
      if (errEl) errEl.textContent = (e && e.message) ? e.message : I18n.t('panel.pwChangeFail');
    } finally {
      btn.disabled = false;
    }
  }

  // Wire buttons
  document.addEventListener('DOMContentLoaded', function () {
    var clearBtn = document.getElementById('clear-log-btn');
    if (clearBtn) clearBtn.addEventListener('click', clearLogs);
    var exportBtn = document.getElementById('export-log-btn');
    if (exportBtn) exportBtn.addEventListener('click', exportLogs);
    // Diagnostics modal
    var diagBtn = document.getElementById('diag-log-btn');
    if (diagBtn) diagBtn.addEventListener('click', showDiagModal);
    var diagClose = document.getElementById('diag-close');
    if (diagClose) diagClose.addEventListener('click', hideDiagModal);
    var diagOverlay = document.getElementById('diag-overlay');
    if (diagOverlay) diagOverlay.addEventListener('click', function (e) {
      if (e.target === this) hideDiagModal();
    });
    // Device info modal
    var trigger = document.getElementById('device-info-trigger');
    if (trigger) trigger.addEventListener('click', showDeviceInfoModal);
    var closeBtn = document.getElementById('device-info-close');
    if (closeBtn) closeBtn.addEventListener('click', hideDeviceInfoModal);
    var overlay = document.getElementById('device-info-overlay');
    if (overlay) overlay.addEventListener('click', function (e) {
      if (e.target === this) hideDeviceInfoModal();
    });
    // Change password modal — click nickname opens it
    var nickEl = document.getElementById('p-nick');
    if (nickEl) {
      nickEl.style.cursor = 'pointer';
      nickEl.addEventListener('click', openChangePw);
    }
    var cpClose = document.getElementById('change-pw-close');
    if (cpClose) cpClose.addEventListener('click', hideChangePw);
    var cpOverlay = document.getElementById('change-pw-overlay');
    if (cpOverlay) cpOverlay.addEventListener('click', function (e) {
      if (e.target === this) hideChangePw();
    });
    var cpSubmit = document.getElementById('change-pw-submit');
    if (cpSubmit) cpSubmit.addEventListener('click', submitChangePw);
    // Enter key submits inside change-pw overlay
    var cpCard = cpOverlay ? cpOverlay.querySelector('.overlay-card') : null;
    if (cpCard) cpCard.addEventListener('keydown', function (e) {
      if (e.key === 'Enter' && cpOverlay.classList.contains('active')) submitChangePw();
    });
  });

  return {
    init: init, show: show, cleanup: cleanup,
    addLog: addLog
  };
})();
