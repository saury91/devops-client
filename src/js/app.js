// Main entry point — i18n init, login/auto-login flow, view switching.
var App = (function () {
  'use strict';

  async function init() {
    // Init background animation
    Background.init();

    // Default to Chinese, use system language only if it's explicitly supported
    try {
      var rustLang = await API.getLang();
      // Only accept en if system is explicitly English; otherwise default zh
      var lang = (rustLang === 'en') ? 'en' : 'zh';
      await I18n.init(lang);
    } catch (_) {
      await I18n.init('zh');
    }

    applyTranslations();

    // Window control buttons
    document.querySelectorAll('.btn-minimize').forEach(function (btn) {
      btn.addEventListener('click', function () { API.minimizeWindow(); });
    });
    document.querySelectorAll('.btn-close').forEach(function (btn) {
      // Panel close (×) hides window to dock/tray; login has btn-hide for hide
      btn.addEventListener('click', function () { API.hideWindow(); });
    });
    document.querySelectorAll('.btn-hide').forEach(function (btn) {
      btn.addEventListener('click', function () { API.hideWindow(); });
    });

    // Custom titlebar drag for login, auto-login and panel views
    ['#login-view', '#auto-login-view', '#panel-view'].forEach(function (selector) {
      var titlebar = document.querySelector(selector + ' .term-titlebar');
      if (titlebar) {
        titlebar.addEventListener('mousedown', function (e) {
          if (e.target.closest('.win-actions')) return;
          API.startDrag();
        });
      }
    });

    // Init all views
    LoginView.init();
    Panel.init();
    Settings.init();

    // Wire panel quit button → logout (return to login view, clear session)
    document.getElementById('quit-btn-panel').addEventListener('click', logout);

    // Listen for device revoked
    API.onRevoked(function () {
      alert(I18n.t('error.revoked'));
      logout();
    });

    // Listen for connection lost (heartbeat failed 3 times)
    API.onConnectionLost(function () {
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

          // Persist fresh token and update login time
          cfg.token = newToken;
          cfg.login_at = LoginView.formatLoginTime(new Date());
          await API.saveConfig(cfg);

          // Start proxy + heartbeat
          port = await API.getProxyPort();
          if (!port) port = await API.startProxy(fp);
          await API.startHeartbeat(cfg.server_url, fp);
        } catch (e) {
          autoError = String(e && e.message ? e.message : e);
        }

        // Ensure the auto-login page is visible for at least 3 seconds
        var elapsed = Date.now() - autoLoginStart;
        var remaining = Math.max(0, 3000 - elapsed);
        await sleep(remaining);

        if (autoError) {
          // Auto-login failed: switch to the login form with the reason
          switchView('login', {
            username: cfg.username || '',
            error: I18n.t('login.autoLoginFailed') + ': ' + autoError
          });
          return;
        }

        // Success: go to panel
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
    var elements = document.querySelectorAll('[data-i18n]');
    elements.forEach(function (el) {
      var key = el.getAttribute('data-i18n');
      if (!key) return;
      var text = I18n.t(key);
      // Buttons with a .btn-text label should keep their shimmer/icons.
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
      gearBtn.style.display = 'none';
      API.resizeWindow(360, 624);
      Background.stop();
      Panel.show(state);
    } else if (name === 'auto-login') {
      loginView.classList.remove('active');
      panelView.classList.remove('active');
      html.classList.remove('panel-active', 'login-active');
      html.classList.add('auto-login-active');
      autoLoginView.classList.add('active');
      gearBtn.style.display = 'none';
      API.resizeWindow(360, 320);
      Background.stop();
    } else if (name === 'login') {
      panelView.classList.remove('active');
      autoLoginView.classList.remove('active');
      html.classList.remove('panel-active', 'auto-login-active');
      html.classList.add('login-active');
      loginView.classList.add('active');
      gearBtn.style.display = 'flex';
      API.resizeWindow(360, 320);
      Background.stop();
      if (state) {
        LoginView.applyState(state);
      }
    }
  }

  // Cleanup: stop proxy + heartbeat, clear config, reset form fields (optional)
  async function _doLogout(clearForm) {
    try { await API.stopProxy(); } catch (_) {}
    try { await API.stopHeartbeat(); } catch (_) {}
    var cfg = await API.loadConfig();
    if (cfg) {
      if (cfg.server_url && cfg.token) {
        try { await API.serverLogout(cfg.server_url, cfg.token); } catch (_) {}
      }
      cfg.token = '';
      await API.saveConfig(cfg);
    }
    document.getElementById('msg-label').textContent = '';
    var loginBtn = document.getElementById('login-btn');
    loginBtn.disabled = false;
    loginBtn.classList.remove('is-loading');
    loginBtn.querySelector('.btn-text').textContent = I18n.t('login.signIn');
    if (clearForm) {
      document.getElementById('user-input').value = '';
      document.getElementById('pass-input').value = '';
    }
    switchView('login');
  }

  async function logout() {
    await _doLogout(true);  // Manual: clear username/password
  }

  async function quitApp() {
    try { await API.stopProxy(); } catch (_) {}
    try { await API.stopHeartbeat(); } catch (_) {}
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
