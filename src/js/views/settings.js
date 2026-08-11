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

  async function save() {
    var url = normalizeServerUrl(document.getElementById('settings-url').value);
    var lang = _langSelect.value;
    var statusEl = document.getElementById('settings-conn-status');

    try {
      var cfg = await API.loadConfig() || { server_url: '', token: '', language: '' };
      cfg.server_url = url;
      cfg.language = lang;
      await API.saveConfig(cfg);

      if (lang !== I18n.lang()) {
        await I18n.load(lang);
        App.applyTranslations();
        document.documentElement.lang = lang;
      }

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

  function showFeedback(msg, isError) {
    var el = document.getElementById('settings-conn-status');
    if (!el) return;
    el.textContent = msg;
    el.className = 'conn-status ' + (isError ? 'error' : 'success');
  }

  return { init: init, show: show, hide: hide };
})();
