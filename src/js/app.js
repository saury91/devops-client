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

  // ---- 更新检查 / 安装 ----
  //
  // 三条约束：
  //  1) 启动时的自动检查只提示、不下载。静默替换二进制并重启会打断正在进行的工作 ——
  //     旧实现是在用户毫不知情的情况下就跑完了 download_and_install。
  //  2) 下载必须有可见进度。安装包以十兆计，没有反馈的等待会被当成卡死。
  //  3) 失败不能只写 console。这个应用没有打包器，终端用户打不开开发者工具，
  //     console.error 那句只有我们自己看得到。
  var _update = { checking: false, installing: false, rid: null, version: '', downloaded: 0 };

  function updateStatus(text, kind) {
    var el = document.getElementById('settings-update-status');
    if (!el) return;
    el.textContent = text || '';
    el.className = 'conn-status' + (kind ? ' ' + kind : '');
  }

  function updateInstallVisible(visible) {
    var btn = document.getElementById('settings-install-update');
    if (btn) btn.style.display = visible ? 'inline-block' : 'none';
  }

  function humanSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  }

  // manual = 用户在设置页主动点了「检查更新」。主动时必须明确回答「已是最新」或
  // 「检查失败」；自动检查则只在真能装到东西时才打扰用户。
  async function checkUpdate(manual) {
    var T = window.__TAURI__;
    if (!T || !T.core || !T.core.invoke) {
      // 直接用浏览器打开 index.html 调试时会走到这里。
      if (manual) updateStatus(I18n.t('update.unsupported'), 'error');
      return null;
    }
    if (_update.checking || _update.installing) return null;

    _update.checking = true;
    if (manual) updateStatus(I18n.t('update.checking'), 'testing');

    try {
      var metadata = await T.core.invoke('plugin:updater|check');
      // 没有可用更新时插件返回 null；rid 是后续下载要用的资源句柄。
      if (!metadata || !metadata.rid) {
        _update.rid = null;
        updateInstallVisible(false);
        if (manual) updateStatus(I18n.t('update.upToDate'), 'success');
        return null;
      }

      _update.rid = metadata.rid;
      _update.version = metadata.version || '';
      var label = I18n.t('update.found', { version: _update.version });
      updateInstallVisible(true);
      updateStatus(label, 'success');
      if (!manual) showToast(label, 'info');
      return metadata;
    } catch (e) {
      console.error('checkUpdate failed:', e);
      // 更新服务不可达（离线、内网、尚未发布）不是需要报警的状态，自动检查时保持安静；
      // 但用户主动点了，就必须给答复。
      if (manual) updateStatus(I18n.t('update.checkFailed') + ': ' + String(e), 'error');
      return null;
    } finally {
      _update.checking = false;
    }
  }

  async function installUpdate() {
    var T = window.__TAURI__;
    if (!T || !T.core || !T.core.invoke || _update.installing) return;
    // 启动时的检查可能还没跑完、或当时正好离线，这里补一次检查再决定能否继续。
    if (!_update.rid && !(await checkUpdate(true))) return;

    _update.installing = true;
    _update.downloaded = 0;
    updateInstallVisible(false);
    updateStatus(I18n.t('update.downloading'), 'testing');

    try {
      var channel = new T.core.Channel();
      channel.onmessage = function (event) {
        if (!event) return;
        if (event.event === 'Started') {
          var total = (event.data && event.data.contentLength) || 0;
          updateStatus(
            total ? I18n.t('update.downloadingOf', { size: humanSize(total) }) : I18n.t('update.downloading'),
            'testing'
          );
        } else if (event.event === 'Progress') {
          _update.downloaded += (event.data && event.data.chunkLength) || 0;
          updateStatus(I18n.t('update.downloaded', { size: humanSize(_update.downloaded) }), 'testing');
        } else if (event.event === 'Finished') {
          // 下载结束后还有校验与替换安装包两步，它们没有进度回调，给一句明确的等待提示。
          updateStatus(I18n.t('update.installing'), 'testing');
        }
      };

      await T.core.invoke('plugin:updater|download_and_install', { onEvent: channel, rid: _update.rid });

      // 重启才会真正切到新版本。这里不弹 toast：重启会让窗口立即消失，吐司来不及被看见，
      // 而设置页上的状态文字用户已经看到了。
      updateStatus(I18n.t('update.restarting'), 'testing');
      await T.core.invoke('plugin:process|restart');
    } catch (e) {
      console.error('installUpdate failed:', e);
      _update.installing = false;
      updateInstallVisible(true);
      updateStatus(I18n.t('update.installFailed') + ': ' + String(e), 'error');
      showToast(I18n.t('update.installFailed'), 'error');
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

    // 后台检查更新（best-effort）。只提示，不下载 —— 安装由用户在设置页确认。
    checkUpdate(false);

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
          if (e.target.closest('button')) return; // 标题栏按钮（如齿轮）不应触发拖拽
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
      _doLogout(false); // 被动退出：回显账号与密码
    });

    // Listen for connection lost (heartbeat failed 3 times)
    API.onConnectionLost(function () {
      showToast(I18n.t('error.connectionLost'), 'error');
      _doLogout(false); // 被动退出：回显账号与密码
    });

    // ---- Startup: choose auto-login view or login view ----
    try {
      var fp = await API.getFingerprint();
      var cfg = await API.loadConfig();

      var hasPreviousLogin = cfg && cfg.server_url && cfg.token;

      if (hasPreviousLogin) {
        // 旧版本客户端的登录不登记设备证书，升级上来的配置里没有 cert_registered 标记。
        // 这种安装不能静默续登：否则设备会长期停在「已登录、服务端却没有它的证书」的状态，
        // 所以清掉会话回到登录页（账号密码沿用已保存值，用户只需再点一次登录），
        // 由交互式登录路径补做证书登记。
        if (!cfg.cert_registered) {
          await _doLogout(false, I18n.t('login.certReloginRequired'));
          return;
        }

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
          // 走和被动退出同一条清理路径：失败点可能出现在后半程（startHeartbeat 或 saveConfig
          // 抛错），此时本地代理已经起来了；只清 token 会把代理和心跳线程留在后台占着端口，
          // 而界面已经回到登录页，用户再登录一次就会起第二个。
          // _doLogout 会停服务、清 token、回填账号密码并带着提示回到登录页。
          await _doLogout(false, I18n.t('login.autoLoginFailed') + ': ' + autoError);
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

    // No previous login / error: show the normal login form directly.
    // 已保存的账号密码默认回填，用户不必重新输入（主动退出时会清掉保存的密码）。
    switchView('login', cfg ? { username: cfg.username || '', password: cfg.password || '' } : undefined);
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

  // 登出会停代理与心跳、写 config、切视图。这些调用可能并发到来（设备被撤销、连接丢失、
  // 用户手动退出、自动登录失败），并行执行时两条流程会交叉写配置、切换视图，用户最终停在哪一页
  // 取决于谁后跑完。这里把每次调用排队串行执行 —— 不合并，因为两者语义不同：
  // 手动退出要清掉保存的密码，被动退出要回填账号密码。
  var _logoutQueue = Promise.resolve();

  function _doLogout(clearForm, notice) {
    var run = function () { return doLogout(clearForm, notice); };
    // 无论前一次成功还是失败都继续排队，一次登出失败不该让后续登出永远排不上。
    var queued = _logoutQueue.then(run, run);
    _logoutQueue = queued.then(function () {}, function () {});
    return queued;
  }

  // Cleanup: stop proxy + heartbeat, clear config, reset form fields (optional)
  // `notice` 显示在登录页上，用于说明这次为什么被退回登录（如升级后需要重新登录一次）。
  async function doLogout(clearForm, notice) {
    try { await API.stopProxy(); } catch (e) { console.error('stopProxy failed:', e); }
    try { await API.stopHeartbeat(); } catch (e) { console.error('stopHeartbeat failed:', e); }
    Panel.cleanup();
    var cfg = await API.loadConfig();
    var savedUsername = '';
    var savedPassword = '';
    if (cfg) {
      if (cfg.server_url && cfg.token) {
        try { await API.serverLogout(cfg.server_url, cfg.token); } catch (e) { console.error('serverLogout failed:', e); }
      }
      if (clearForm) {
        // 主动退出：清除已保存密码，不回显
        cfg.password = '';
      } else {
        // 被动退出：回显已保存的账号与密码
        savedUsername = cfg.username || '';
        savedPassword = cfg.password || '';
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
      switchView('login', notice ? { error: notice } : undefined);
    } else {
      // 被动退出、被要求重新登录：回显已保存的账号与密码
      switchView('login', { username: savedUsername, password: savedPassword, error: notice });
    }
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

  // 会话已失效但用户没主动退出时使用（如设置页改了服务器地址）：停服务、清 token、
  // 回登录页并回显账号密码，与"被动退出"同一条路径。
  function invalidateSession(notice) {
    return _doLogout(false, notice);
  }

  return {
    init: init,
    switchView: switchView,
    applyTranslations: applyTranslations,
    logout: logout,
    quitApp: quitApp,
    invalidateSession: invalidateSession,
    checkUpdate: checkUpdate,
    installUpdate: installUpdate
  };
})();

document.addEventListener('DOMContentLoaded', function () { App.init(); });
