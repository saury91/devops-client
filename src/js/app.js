// Main entry point — i18n init, login/auto-login flow, view switching.
var App = (function () {
  'use strict';

  var _dragTimer = 0;

  function showToast(message, type) {
    var container = document.getElementById('toast-container');
    if (!container) {
      container = document.createElement('div');
      container.id = 'toast-container';
      container.style.cssText = 'position:fixed;top:12px;left:50%;transform:translateX(-50%);z-index:9999;display:flex;flex-direction:column;gap:8px;pointer-events:none;';
      document.body.appendChild(container);
    }
    var el = document.createElement('div');
    var color = type === 'error' ? '#FF5E5B' : '#00E5FF';
    el.style.cssText = 'padding:8px 14px;border-radius:6px;background:rgba(15,27,46,0.95);border:1px solid ' + color + ';color:' + color + ';font-size:12px;box-shadow:0 4px 16px rgba(0,0,0,0.4);pointer-events:auto;opacity:0;transition:opacity 0.2s;';
    el.textContent = message;
    container.appendChild(el);
    requestAnimationFrame(function () { el.style.opacity = '1'; });
    setTimeout(function () {
      el.style.opacity = '0';
      setTimeout(function () { if (el.parentNode) el.parentNode.removeChild(el); }, 200);
    }, 3500);
  }

  async function checkForUpdate() {
    try {
      var updater = await import('@tauri-apps/plugin-updater');
      var process = await import('@tauri-apps/plugin-process');
      var update = await updater.check();
      if (update) {
        showToast(I18n.t('update.found') + ' ' + update.version, 'info');
        await update.downloadAndInstall(function (event) {
          switch (event.event) {
            case 'Started':
            case 'Progress':
            case 'Finished':
              break;
          }
        });
        showToast(I18n.t('update.installing'), 'info');
        await process.relaunch();
      }
    } catch (e) {
      // Best-effort update check; ignore failures.
    }
  }

  async function init() {
    // Init background animation (best-effort)
    try { Background.init(); } catch (_) {}

    // Default to Chinese, use system language only if it's explicitly supported
    try {
      var rustLang = await API.getLang();
      var lang = (rustLang === 'en') ? 'en' : 'zh';
      try {
        var cfgLang = await API.loadConfig();
        if (cfgLang && cfgLang.language) {
          lang = cfgLang.language;
        }
      } catch (_) {}
      await I18n.init(lang);
    } catch (_) {
      try { await I18n.init('zh'); } catch (_) {}
    }

    // Set HTML lang attribute dynamically
    document.documentElement.lang = I18n.lang();

    applyTranslations();

    // Check for app updates in the background (best-effort)
    checkForUpdate();

    // Window control buttons
    document.querySelectorAll('.btn-minimize').forEach(function (btn) {
      btn.addEventListener('click', function () { API.minimizeWindow(); });
    });
    document.querySelectorAll('.btn-close').forEach(function (btn) {
      btn.addEventListener('click', function () { API.hideWindow(); });
    });
    document.querySelectorAll('.btn-hide').forEach(function (btn) {
      btn.addEventListener('click', function () { API.hideWindow(); });
    });

    // Custom titlebar drag — debounced to avoid excessive IPC calls
    ['#login-view', '#auto-login-view', '#panel-view'].forEach(function (selector) {
      var titlebar = document.querySelector(selector + ' .term-titlebar');
      if (titlebar) {
        titlebar.addEventListener('mousedown', function (e) {
          if (e.target.closest('.win-actions')) return;
          // Debounce: at most one startDrag per 200ms
          if (_dragTimer) return;
          _dragTimer = setTimeout(function () { _dragTimer = 0; }, 200);
          API.startDrag();
        });
      }
    });

    // Init all views
    LoginView.init();
    Panel.init();
    Settings.init();

    // Listen for agent ping from PC/browser
    API.onProxyPing(function () { Panel.addLog('PING', true); });

    // Listen for heartbeat events from Rust backend
    API.onHeartbeatOk(function () { Wave.heartbeatOk(); });
    API.onHeartbeatFail(function () { Wave.heartbeatFail(); });

    // Wire panel quit button → logout
    document.getElementById('quit-btn-panel').addEventListener('click', logout);

    // Listen for device revoked
    API.onRevoked(function () {
      showToast(I18n.t('error.revoked'), 'error');
      logout();
    });

    // Listen for connection lost (heartbeat failed 3 times)
    API.onConnectionLost(function () {
      showToast(I18n.t('error.connectionLost'), 'error');
      _doLogout(false); // Keep username for quick re-entry
    });

    // ---- Startup: choose auto-login view or login view ----
    try {
      var fp = await API.getFingerprint();
      var cfg = await API.loadConfig();

      var hasPreviousLogin = cfg && cfg.server_url && cfg.token;

      if (hasPreviousLogin) {
        // Show the dedicated auto-login page directly
        renderAutoLoginUser(cfg.nickname || cfg.username || '-');
        switchView('auto-login');

        var autoLoginStart = Date.now();
        var autoError = null;
        var newToken = '';
        var port = null;

        try {
          var autoResult = await API.autoLogin(cfg.server_url, fp);
          newToken = autoResult && autoResult.token ? autoResult.token : '';
          if (!newToken) {
            throw new Error(I18n.t('login.autoLoginFailed') + ': empty token');
          }

          // Start proxy + heartbeat BEFORE saving the new token
          port = await API.getProxyPort();
          if (!port) port = await API.startProxy(fp);
          await API.startHeartbeat(cfg.server_url, fp);

          // Persist fresh token only after services started
          cfg.token = newToken;
          cfg.login_at = LoginView.formatLoginTime(new Date());
          await API.saveConfig(cfg);
        } catch (e) {
          autoError = String(e && e.message ? e.message : e);
        }

        // Show error during splash if it occurred
        if (autoError) {
          var errEl = document.getElementById('auto-login-error');
          if (errEl) {
            errEl.textContent = I18n.t('login.autoLoginFailed') + ': ' + autoError;
            errEl.classList.add('visible');
          }
        }

        // Ensure the auto-login page is visible for at least 3 seconds
        var elapsed = Date.now() - autoLoginStart;
        var remaining = Math.max(0, 3000 - elapsed);
        await sleep(remaining);

        if (autoError) {
          // Clear the stale token so next startup goes straight to login
          try {
            cfg.token = '';
            await API.saveConfig(cfg);
          } catch (_) {}

          switchView('login', {
            username: cfg.username || '',
            error: I18n.t('login.autoLoginFailed') + ': ' + autoError
          });
          return;
        }

        // Success: go to panel (hardware info will be fetched locally)
        switchView('panel', {
          serverUrl: cfg.server_url,
          fingerprint: fp,
          port: port,
          token: newToken,
          auto: true,
          username: cfg.username || '',
          nickname: cfg.nickname || cfg.username || '',
          loginAt: cfg.login_at || '-'
        });
        return;
      }
    } catch (e) {
      // Fall through to login form
    }

    // No previous login / error: show the normal login form directly
    switchView('login');
  }

  function sleep(ms) {
    return new Promise(function (resolve) { setTimeout(resolve, ms); });
  }

  function renderAutoLoginUser(name) {
    var nickEl = document.getElementById('auto-login-nick');
    var imgEl = document.getElementById('auto-login-avatar');
    var fallbackEl = document.getElementById('auto-login-avatar-fallback');

    if (nickEl) nickEl.textContent = name;
    if (imgEl && fallbackEl) {
      var initial = name.charAt(0).toUpperCase();
      imgEl.src = AvatarUtil.generateDefaultAvatar(initial);
      imgEl.style.display = 'block';
      fallbackEl.style.display = 'none';
    }
  }

  function applyTranslations() {
    document.title = I18n.t('login.title');
    // Also translate title attributes
    document.querySelectorAll('[data-i18n-title]').forEach(function (el) {
      el.setAttribute('title', I18n.t(el.getAttribute('data-i18n-title')));
    });
    var elements = document.querySelectorAll('[data-i18n]');
    elements.forEach(function (el) {
      var key = el.getAttribute('data-i18n');
      if (!key) return;
      var text = I18n.t(key);
      if (el.tagName === 'BUTTON') {
        var label = el.querySelector('.btn-text');
        if (label) {
          label.textContent = text;
          return;
        }
      }
      if ((el.tagName === 'INPUT' && (el.type === 'text' || el.type === 'password')) ||
          el.tagName === 'TEXTAREA') {
        el.placeholder = text;
      } else if (el.tagName === 'OPTION') {
        el.textContent = text;
      } else {
        el.textContent = text;
      }
    });
  }

  function switchView(name, state) {
    var loginView = document.getElementById('login-view');
    var autoLoginView = document.getElementById('auto-login-view');
    var panelView = document.getElementById('panel-view');
    var gearBtn = document.getElementById('gear-btn');
    var html = document.documentElement;

    if (name === 'panel') {
      loginView.classList.remove('active');
      autoLoginView.classList.remove('active');
      html.classList.remove('login-active', 'auto-login-active');
      html.classList.add('panel-active');
      panelView.classList.add('active');
      if (gearBtn) gearBtn.style.display = 'none';
      API.resizeWindow(360, 624);
      Background.stop();
      Panel.show(state);
    } else if (name === 'auto-login') {
      loginView.classList.remove('active');
      panelView.classList.remove('active');
      html.classList.remove('panel-active', 'login-active');
      html.classList.add('auto-login-active');
      autoLoginView.classList.add('active');
      if (gearBtn) gearBtn.style.display = 'none';
      API.resizeWindow(360, 320);
      Background.stop();
    } else if (name === 'login') {
      panelView.classList.remove('active');
      autoLoginView.classList.remove('active');
      html.classList.remove('panel-active', 'auto-login-active');
      html.classList.add('login-active');
      loginView.classList.add('active');
      if (gearBtn) gearBtn.style.display = 'flex';
      API.resizeWindow(360, 320);
      Background.stop();
      // Reset login button state
      var loginBtn = document.getElementById('login-btn');
      if (loginBtn) {
        loginBtn.disabled = false;
        loginBtn.classList.remove('is-loading');
        var btnText = loginBtn.querySelector('.btn-text');
        if (btnText) btnText.textContent = I18n.t('login.signIn');
      }
      if (state) {
        LoginView.applyState(state);
      }
    }
  }

  // Cleanup: stop proxy + heartbeat, clear config, reset form fields (optional)
  async function _doLogout(clearForm) {
    try { await API.stopProxy(); } catch (e) { console.error('stopProxy failed:', e); }
    try { await API.stopHeartbeat(); } catch (e) { console.error('stopHeartbeat failed:', e); }
    Panel.cleanup();
    var cfg = await API.loadConfig();
    if (cfg) {
      if (cfg.server_url && cfg.token) {
        try { await API.serverLogout(cfg.server_url, cfg.token); } catch (e) { console.error('serverLogout failed:', e); }
      }
      cfg.token = '';
      try { await API.saveConfig(cfg); } catch (e) { console.error('saveConfig on logout failed:', e); }
    }
    document.getElementById('msg-label').textContent = '';
    var loginBtn = document.getElementById('login-btn');
    if (loginBtn) {
      loginBtn.disabled = false;
      loginBtn.classList.remove('is-loading');
      var btnTextEl = loginBtn.querySelector('.btn-text');
      if (btnTextEl) btnTextEl.textContent = I18n.t('login.signIn');
    }
    if (clearForm) {
      var userInput = document.getElementById('user-input');
      var passInput = document.getElementById('pass-input');
      if (userInput) userInput.value = '';
      if (passInput) passInput.value = '';
    }
    switchView('login');
  }

  async function logout() {
    await _doLogout(true);  // Manual: clear username/password
  }

  async function quitApp() {
    try { await API.stopProxy(); } catch (e) { console.error('quit: stopProxy failed:', e); }
    try { await API.stopHeartbeat(); } catch (e) { console.error('quit: stopHeartbeat failed:', e); }
    Panel.cleanup();
    API.quit();
  }

  return {
    init: init,
    switchView: switchView,
    applyTranslations: applyTranslations,
    logout: logout,
    quitApp: quitApp
  };
})();

document.addEventListener('DOMContentLoaded', function () { App.init(); });
