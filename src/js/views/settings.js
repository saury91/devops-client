// Settings panel — server URL, language, connection test.
var Settings = (function () {
  'use strict';

  var _visible = false;
  var _langSelect;

  function init() {
    document.getElementById('gear-btn').addEventListener('click', toggle);

    document.getElementById('settings-close').addEventListener('click', hide);
    document.getElementById('settings-save').addEventListener('click', save);
    _langSelect = document.getElementById('lang-select');

    // Connection test
    var testBtn = document.getElementById('settings-test-conn');
    if (testBtn) testBtn.addEventListener('click', testConnection);

    // App update — 安装入口只在这里，启动时的自动检查不会擅自下载。
    var checkBtn = document.getElementById('settings-check-update');
    if (checkBtn) checkBtn.addEventListener('click', function () { App.checkUpdate(true); });
    var installBtn = document.getElementById('settings-install-update');
    if (installBtn) installBtn.addEventListener('click', function () { App.installUpdate(); });

    document.getElementById('settings-overlay').addEventListener('click', function (e) {
      if (e.target === this) hide();
    });

    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && _visible) hide();
    });
  }

  function show() {
    _visible = true;

    var urlInput = document.getElementById('settings-url');
    API.loadConfig().then(function (cfg) {
      urlInput.value = (cfg && cfg.server_url) ? cfg.server_url : '';
    });

    _langSelect.value = I18n.lang();

    // 版本号取自 Tauri（即 tauri.conf.json 里的 version），而不是前端再硬编码一份 ——
    // 硬编码的副本总会在某次发版时被忘记同步，而"关于页显示的版本"恰恰是排查时最被信任的信息。
    var versionEl = document.getElementById('settings-version');
    var T = window.__TAURI__;
    if (versionEl) {
      if (T && T.app && T.app.getVersion) {
        T.app.getVersion()
          .then(function (v) { versionEl.textContent = v; })
          .catch(function () { versionEl.textContent = '-'; });
      } else {
        versionEl.textContent = '-';
      }
    }

    document.getElementById('settings-overlay').classList.add('active');
  }

  function hide() {
    _visible = false;
    document.getElementById('settings-overlay').classList.remove('active');
  }

  function toggle() {
    _visible ? hide() : show();
  }

  // 用户常省略协议前缀（如 192.168.1.100:8080），保存前自动补 http://
  function normalizeServerUrl(raw) {
    var url = (raw || '').trim();
    if (!url) return url;
    if (!/^https?:\/\//i.test(url)) {
      url = 'http://' + url;
    }
    return url;
  }

  // 地址末尾的斜杠和大小写不该算作"改了地址"（否则用户重存一次就会被登出）
  function comparableUrl(url) {
    return (url || '').replace(/\/+$/, '').toLowerCase();
  }

  async function save() {
    var url = normalizeServerUrl(document.getElementById('settings-url').value);
    var lang = _langSelect.value;
    var statusEl = document.getElementById('settings-conn-status');

    try {
      var cfg = await API.loadConfig() || { server_url: '', token: '', language: '' };
      var previousUrl = cfg.server_url || '';

      // 先把语言切过去：后续无论走哪条分支，界面文案（含退回登录页的提示）都是新语言。
      if (lang !== I18n.lang()) {
        await I18n.load(lang);
        App.applyTranslations();
        document.documentElement.lang = lang;
      }

      // 地址变了、而且当前还持有会话：token 是旧服务端签发的，在新服务端上没有任何意义，
      // 心跳线程也仍然抓着旧地址在跑。必须在改写 server_url 之前先拆掉旧会话 ——
      // server_logout 要用「旧地址 + 旧 token」才能在签发它的服务端上真正注销，
      // 否则旧凭据会被发到新地址、旧服务端也收不到登出。
      var addressChanged =
        previousUrl && cfg.token && comparableUrl(url) !== comparableUrl(previousUrl);

      if (addressChanged) {
        // 停代理/心跳、清 token、带提示退回登录页（账号密码保留并回填）。
        hide();
        await App.invalidateSession(I18n.t('settings.serverChanged'));
        // 会话已清空（此时 server_url 仍为旧值），再把地址与语言落到新值。
        var cleared = await API.loadConfig() || {};
        cleared.server_url = url;
        cleared.language = lang;
        await API.saveConfig(cleared);
        return;
      }

      cfg.server_url = url;
      cfg.language = lang;
      await API.saveConfig(cfg);
      hide();

      var btn = document.getElementById('settings-save');
      var orig = btn.querySelector('.btn-text').textContent;
      btn.querySelector('.btn-text').textContent = I18n.t('settings.saved');
      btn.style.color = '#00E5FF';
      setTimeout(function () {
        btn.querySelector('.btn-text').textContent = orig;
        btn.style.color = '';
      }, 1500);
    } catch (e) {
      console.error('settings save failed:', e);
      if (statusEl) {
        statusEl.textContent = (e && e.message) ? e.message : I18n.t('error.serverError');
        statusEl.className = 'conn-status error';
      }
    }
  }

  async function testConnection() {
    var url = normalizeServerUrl(document.getElementById('settings-url').value);
    var statusEl = document.getElementById('settings-conn-status');
    if (!url) {
      if (statusEl) { statusEl.textContent = I18n.t('error.noServerUrl'); statusEl.className = 'conn-status error'; }
      return;
    }

    if (statusEl) { statusEl.textContent = I18n.t('settings.testing'); statusEl.className = 'conn-status testing'; }

    try {
      var resp = await API.testConnection(url);
      if (resp && resp.ok) {
        if (statusEl) {
          statusEl.textContent = I18n.t('settings.connOk') + ' (' + resp.latency + 'ms)';
          statusEl.className = 'conn-status success';
        }
      } else {
        if (statusEl) { statusEl.textContent = I18n.t('settings.connFail') + ' (HTTP ' + (resp && resp.status ? resp.status : '?') + ')'; statusEl.className = 'conn-status error'; }
      }
    } catch (e) {
      if (statusEl) { statusEl.textContent = I18n.t('settings.connFail') + ': ' + String(e); statusEl.className = 'conn-status error'; }
    }
  }

  return { init: init, show: show, hide: hide };
})();
