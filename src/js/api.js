// Tauri IPC wrapper — provides a clean API for invoking Rust commands.
var API = (function () {
  'use strict';

  function invoke(cmd, args) {
    args = args || {};
    return window.__TAURI__.core.invoke(cmd, args);
  }

  function post(url, body, token) {
    return fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Session-Id': token || ''
      },
      body: JSON.stringify(body || {})
    }).then(function (r) { return r.json(); });
  }

  return {
    getLang:       function ()       { return invoke('get_lang'); },
    getFingerprint: function ()      { return invoke('get_fingerprint'); },
    loadConfig:    function ()       { return invoke('load_config_cmd'); },
    saveConfig:    function (cfg)    { return invoke('save_config_cmd', { config: cfg }); },
    getHostname:   function ()       { return invoke('get_hostname'); },
    getOsInfo:     function ()       { return invoke('get_os_info'); },
    getUserInfo:   function (url, token) { return invoke('get_user_info', { serverUrl: url, token: token }); },
    post:          post,
    doLogin:       function (url, user, pass, dev) {
      return invoke('do_login', {
        serverUrl: url, username: user, password: pass, deviceName: dev
      });
    },
    serverLogout:  function (url, token) { return invoke('server_logout', { serverUrl: url, token: token }); },
    autoLogin:     function (url, fp) { return invoke('auto_login', { serverUrl: url, fingerprint: fp }); },
    startProxy:    function (fp) { return invoke('start_proxy', { fingerprint: fp }); },
    stopProxy:     function ()       { return invoke('stop_proxy'); },
    getProxyPort:  function ()       { return invoke('get_proxy_port'); },
    resizeWindow:  function (w, h)   { return invoke('resize_window', { width: w, height: h }); },
    minimizeWindow: function ()      { return invoke('minimize_window'); },
    hideWindow:    function ()      { return invoke('hide_window'); },
    startDrag:     function ()      { return invoke('start_drag'); },
    openBrowser:   function (url)    { return invoke('open_browser', { url: url }); },
    openDashboard: function (serverUrl, token, port) { return invoke('open_dashboard', { serverUrl, token, port }); },
    startHeartbeat: function (url, fp) { return invoke('start_heartbeat', { serverUrl: url, fingerprint: fp }); },
    stopHeartbeat: function ()       { return invoke('stop_heartbeat'); },
    onRevoked:     function (cb) {
      if (window.__TAURI__ && window.__TAURI__.event) {
        window.__TAURI__.event.listen('device-revoked', cb);
      }
    },
    onConnectionLost: function (cb) {
      if (window.__TAURI__ && window.__TAURI__.event) {
        window.__TAURI__.event.listen('connection-lost', cb);
      }
    },
    quit: function () {
      return invoke('quit_app');
    }
  };
})();
