// Tauri IPC wrapper — provides a clean API for invoking Rust commands.
var API = (function () {
  'use strict';

  var _unlisteners = {};

  function invoke(cmd, args) {
    args = args || {};
    return window.__TAURI__.core.invoke(cmd, args);
  }

  // 这里原本有一个直接 fetch 到业务服务端的 `post()`，实际没有任何调用点（所有请求都经
  // Rust 侧的 reqwest）。删掉它不只是清理死代码：它是渲染进程里唯一的跨源网络能力，
  // 去掉之后 CSP 的 connect-src 才能收紧到 'self' + IPC，注入脚本也就没有外发通道了。

  function _listenOnce(name, cb) {
    // Clean up any previous listener for this event before registering a new one.
    if (_unlisteners[name]) {
      _unlisteners[name]();
      _unlisteners[name] = null;
    }
    if (!window.__TAURI__ || !window.__TAURI__.event) return Promise.resolve(function () {});
    return window.__TAURI__.event.listen(name, cb).then(function (unlisten) {
      _unlisteners[name] = unlisten;
      return unlisten;
    });
  }

  return {
    getLang:       function ()       { return invoke('get_lang'); },
    getFingerprint: function ()      { return invoke('get_fingerprint'); },
    loadConfig:    function ()       { return invoke('load_config_cmd'); },
    saveConfig:    function (cfg)    { return invoke('save_config_cmd', { config: cfg }); },
    getHostname:   function ()       { return invoke('get_hostname'); },
    getOsInfo:     function ()       { return invoke('get_os_info'); },
    getUserInfo:   function (url, token) { return invoke('get_user_info', { serverUrl: url, token: token }); },
    changePassword: function (url, token, oldPassword, newPassword) {
      return invoke('change_password', { serverUrl: url, token: token, oldPassword: oldPassword, newPassword: newPassword });
    },
    doLogin:       function (url, user, pass, dev) {
      return invoke('do_login', {
        serverUrl: url, username: user, password: pass, deviceName: dev
      });
    },
    serverLogout:  function (url, token) { return invoke('server_logout', { serverUrl: url, token: token }); },
    autoLogin:     function (url, fp) { return invoke('auto_login', { serverUrl: url, fingerprint: fp }); },
    resizeWindow:  function (w, h)   { return invoke('resize_window', { width: w, height: h }); },
    minimizeWindow: function ()      { return invoke('minimize_window'); },
    hideWindow:    function ()      { return invoke('hide_window'); },
    startDrag:     function ()      { return invoke('start_drag'); },
    openBrowser:   function (url)    { return invoke('open_browser', { url: url }); },
    openDashboard: function (serverUrl, token) { return invoke('open_dashboard', { serverUrl, token }); },
    getDashboardUrl: function (serverUrl, token) { return invoke('get_dashboard_url', { serverUrl, token }); },
    startHeartbeat: function (url, fp) { return invoke('start_heartbeat', { serverUrl: url, fingerprint: fp }); },
    stopHeartbeat: function ()       { return invoke('stop_heartbeat'); },
    onRevoked:     function (cb) { return _listenOnce('device-revoked', cb); },
    onIdentityMismatch: function (cb) { return _listenOnce('device-identity-mismatch', cb); },
    onConnectionLost: function (cb) { return _listenOnce('connection-lost', cb); },
    onHeartbeatOk: function (cb) { return _listenOnce('heartbeat-ok', cb); },
    onHeartbeatFail: function (cb) { return _listenOnce('heartbeat-fail', cb); },
    onSessionRenewed: function (cb) { return _listenOnce('session-renewed', cb); },
    getDeviceInfo: function ()     { return invoke('get_device_info'); },
    testConnection: function (url) { return invoke('test_connection', { url: url }); },
    exportLogFile: function (content, path) { return invoke('export_log_file', { content: content, path: path }); },
    readErrorLog: function (maxBytes) { return invoke('read_error_log', { maxBytes: maxBytes }); },
    quit: function () {
      return invoke('quit_app');
    }
  };
})();
