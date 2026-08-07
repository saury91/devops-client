// Login view — username + password.
var LoginView = (function () {
  'use strict';

  function init() {
    document.getElementById('login-btn').addEventListener('click', handleLogin);

    // Password visibility toggle
    var toggled = false;
    document.getElementById('toggle-pass').addEventListener('click', function () {
      toggled = !toggled;
      var input = document.getElementById('pass-input');
      input.type = toggled ? 'text' : 'password';
      document.getElementById('eye-off').style.display = toggled ? 'none' : '';
      document.getElementById('eye-on').style.display = toggled ? '' : 'none';
    });

    // Scoped Enter key listener on the login form body
    var termBody = document.querySelector('#login-view .term-body');
    if (termBody) {
      termBody.addEventListener('keydown', function (e) {
        if (e.key === 'Enter' && document.getElementById('login-view').classList.contains('active')) {
          handleLogin();
        }
      });
    }
  }

  function formatLoginTime(date) {
    return date.getFullYear() + '-' +
      String(date.getMonth() + 1).padStart(2, '0') + '-' +
      String(date.getDate()).padStart(2, '0') + ' ' +
      String(date.getHours()).padStart(2, '0') + ':' +
      String(date.getMinutes()).padStart(2, '0') + ':' +
      String(date.getSeconds()).padStart(2, '0');
  }

  async function handleLogin() {
    var user = document.getElementById('user-input').value.trim();
    var pass = document.getElementById('pass-input').value;
    var btn = document.getElementById('login-btn');
    var msg = document.getElementById('msg-label');

    if (!user || !pass) {
      showMsg(msg, I18n.t('login.fillAll'));
      return;
    }

    // Load server URL from config
    var cfg = await API.loadConfig();
    var url = (cfg && cfg.server_url) ? cfg.server_url.trim() : '';
    if (!url) {
      showMsg(msg, I18n.t('error.noServerUrl'));
      return;
    }

    url = url.replace(/\/+$/, '');
    msg.textContent = '';
    setBtnLoading(btn, true);

    try {
      var hostname = await API.getHostname();
      var result = await API.doLogin(url, user, pass, hostname);

      if (result.status === 'pending') {
        showMsg(msg, result.message || I18n.t('login.pending'));
        return;
      }

      if (result.status === 'ok' && result.token) {
        var userInfo = await API.getUserInfo(url, result.token);
        var nickname = (userInfo && userInfo.nickname) ? userInfo.nickname : user;
        var loginTime = formatLoginTime(new Date());

        // Start proxy + heartbeat BEFORE saving config, so a failure
        // doesn't leave stale token on disk.
        var port = await API.getProxyPort();
        if (!port) port = await API.startProxy(result.fingerprint);
        await API.startHeartbeat(url, result.fingerprint);

        // Now persist config after services are running
        await API.saveConfig({
          server_url: url,
          token: result.token,
          login_at: loginTime,
          username: user,
          password: pass,
          nickname: nickname
        });

        App.switchView('panel', {
          serverUrl: url, fingerprint: result.fingerprint,
          port: port, token: result.token, auto: false,
          username: user,
          nickname: nickname,
          loginAt: loginTime
        });
        return;
      }

      showMsg(msg, result.message || result.token || I18n.t('login.failed'));
    } catch (e) {
      showMsg(msg, I18n.t('login.connFailed') + ': ' + String(e));
    } finally {
      setBtnLoading(btn, false);
    }
  }

  function setBtnLoading(btn, loading) {
    var text = btn.querySelector('.btn-text');
    if (!text) return;
    if (loading) {
      btn.disabled = true;
      btn.classList.add('is-loading');
      text.textContent = I18n.t('login.signingIn');
    } else {
      btn.disabled = false;
      btn.classList.remove('is-loading');
      text.textContent = I18n.t('login.signIn');
    }
  }

  function showMsg(el, text) {
    el.textContent = text;
    el.classList.add('error');
    setTimeout(function () { el.classList.remove('error'); }, 350);
  }

  function applyState(state) {
    var userInput = document.getElementById('user-input');
    var passInput = document.getElementById('pass-input');
    var msg = document.getElementById('msg-label');
    if (state.username && userInput) {
      userInput.value = state.username;
    }
    // 被动退出时回显已保存密码；主动退出（主动登出）不传 password 则留空
    if (passInput) {
      passInput.value = state.password || '';
    }
    if (state.error && msg) {
      showMsg(msg, state.error);
    }
    if (passInput) passInput.focus();
  }

  return { init: init, applyState: applyState, formatLoginTime: formatLoginTime };
})();
