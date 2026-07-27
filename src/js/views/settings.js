// Settings panel — server URL, language, connection test, key management, URL history.
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

    // URL history dropdown
    var urlSelect = document.getElementById('settings-url-history');
    if (urlSelect) urlSelect.addEventListener('change', function () {
      if (urlSelect.value) {
        document.getElementById('settings-url').value = urlSelect.value;
        urlSelect.value = '';
      }
    });

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

    // Populate URL history
    renderUrlHistory();
  }

  function hide() {
    _visible = false;
    document.getElementById('settings-overlay').classList.remove('active');
  }

  function toggle() {
    _visible ? hide() : show();
  }

  async function save() {
    var url = document.getElementById('settings-url').value.trim();
    var lang = _langSelect.value;

    var cfg = await API.loadConfig() || { server_url: '', token: '', language: '' };
    cfg.server_url = url;
    cfg.language = lang;
    await API.saveConfig(cfg);

    // Add URL to history
    if (url) addUrlToHistory(url);

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
  }

  async function testConnection() {
    var url = document.getElementById('settings-url').value.trim();
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

  // --- URL history (localStorage) ---
  function getUrlHistory() {
    try {
      return JSON.parse(localStorage.getItem('devops-url-history') || '[]');
    } catch (_) { return []; }
  }

  function addUrlToHistory(url) {
    var list = getUrlHistory();
    list = list.filter(function (u) { return u !== url; });
    list.unshift(url);
    if (list.length > 10) list.pop();
    localStorage.setItem('devops-url-history', JSON.stringify(list));
  }

  function renderUrlHistory() {
    var sel = document.getElementById('settings-url-history');
    if (!sel) return;
    var list = getUrlHistory();
    sel.innerHTML = '<option value="">' + I18n.t('settings.urlHistory') + '</option>';
    for (var i = 0; i < list.length; i++) {
      sel.innerHTML += '<option value="' + list[i].replace(/"/g, '&quot;') + '">' + list[i] + '</option>';
    }
  }

  return { init: init, show: show, hide: hide };
})();
