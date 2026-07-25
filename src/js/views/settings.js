// Settings panel — server URL, language, extensible for future options.
var Settings = (function () {
  'use strict';

  var _visible = false;
  var _langSelect;

  function init() {
    // Gear icon
    document.getElementById('gear-btn').addEventListener('click', toggle);

    // Close button
    document.getElementById('settings-close').addEventListener('click', hide);

    // Save button
    document.getElementById('settings-save').addEventListener('click', save);

    // Language select
    _langSelect = document.getElementById('lang-select');

    // Click outside to close
    document.getElementById('settings-overlay').addEventListener('click', function (e) {
      if (e.target === this) hide();
    });

    // Escape key
    document.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && _visible) hide();
    });
  }

  function show() {
    _visible = true;

    // Load current values
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

  async function save() {
    var url = document.getElementById('settings-url').value.trim();
    var lang = _langSelect.value;

    // Save server URL to config, preserve fingerprint
    var cfg = await API.loadConfig() || { server_url: '', fingerprint: '', token: '' };
    cfg.server_url = url;
    await API.saveConfig(cfg);

    // Reload locale if language changed
    if (lang !== I18n.lang()) {
      await I18n.load(lang);
      App.applyTranslations();
    }

    hide();

    // Briefly show "Saved" feedback
    var btn = document.getElementById('settings-save');
    var orig = btn.querySelector('.btn-text').textContent;
    btn.querySelector('.btn-text').textContent = I18n.t('settings.saved');
    btn.style.color = '#00E5FF';
    setTimeout(function () {
      btn.querySelector('.btn-text').textContent = orig;
      btn.style.color = '';
    }, 1500);
  }

  return { init: init, show: show, hide: hide };
})();
