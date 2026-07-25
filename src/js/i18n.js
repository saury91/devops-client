// I18n engine — loads locale JSON, supports runtime language switching.
var I18n = (function () {
  'use strict';

  var _lang = 'zh';
  var _strings = {};

  async function init(defaultLang) {
    return load(defaultLang || 'zh');
  }

  async function load(lang) {
    _lang = lang;
    try {
      var resp = await fetch('locales/' + lang + '.json');
      _strings = await resp.json();
    } catch (e) {
      _lang = 'en';
      try {
        var fallback = await fetch('locales/en.json');
        _strings = await fallback.json();
      } catch (_) { _strings = {}; }
    }
  }

  function t(key, params) {
    var s = _strings[key] || key;
    if (params) {
      Object.keys(params).forEach(function (k) {
        s = s.replace('{' + k + '}', params[k]);
      });
    }
    return s;
  }

  function lang() { return _lang; }

  return { init: init, load: load, t: t, lang: lang };
})();
