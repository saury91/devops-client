// Main entry point — i18n init, login/auto-login flow, view switching.
var App = (function () {
  'use strict';

  var _dragTimer = 0;
  var _isDev = false;      // 是否为本地调试构建（tauri dev）
  var _revealed = false;   // 主窗口是否已显示过（visible:false 启动，首屏就绪后只显示一次）

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

    // 调试构建打上 DEV 标识，免得和正式安装的那一份混淆
    await detectDevBuild();

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

    // Listen for heartbeat events from Rust backend
    API.onHeartbeatOk(function () { Wave.heartbeatOk(); });
    API.onHeartbeatFail(function () { Wave.heartbeatFail(); });

    // Wire panel quit button → logout。包一层是因为 logout 现在收 options 对象，
    // 直接把函数交给 addEventListener 会把 click 事件当成 options 传进去。
    document.getElementById('quit-btn-panel').addEventListener('click', function () { logout(); });

    // Listen for device revoked
    API.onRevoked(function () {
      showToast(I18n.t('error.revoked'), 'error');
      _doLogout();
    });

    // Listen for device identity mismatch: the session no longer belongs to this machine.
    // A security event rather than a network failure, so it gets its own message.
    API.onIdentityMismatch(function () {
      showToast(I18n.t('error.identityMismatch'), 'error');
      _doLogout();
    });

    // Listen for connection lost (heartbeat failed 3 times)
    API.onConnectionLost(function () {
      showToast(I18n.t('error.connectionLost'), 'error');
      _doLogout();
    });

    // ---- Startup: choose auto-login view or login view ----
    try {
      var fp = await API.getFingerprint();
      var cfg = await API.loadConfig();

      var hasPreviousLogin = cfg && cfg.server_url && cfg.token;

      if (hasPreviousLogin) {
        // 设备绑定已改由「浏览器侧不可导出密钥的设备证明」完成，登记发生在每次打开工作台时，
        // 不再依赖客户端登录补齐，因此这里可以直接静默续登，历史配置缺失任何标记都不受影响。

        // Show the dedicated auto-login page directly
        renderAutoLoginUser(cfg.nickname || cfg.username || '-');
        switchView('auto-login');

        var autoLoginStart = Date.now();
        var autoError = null;
        var newToken = '';

        try {
          var autoResult = await API.autoLogin(cfg.server_url, fp);
          newToken = autoResult && autoResult.token ? autoResult.token : '';
          if (!newToken) {
            throw new Error(I18n.t('login.autoLoginFailed') + ': empty token');
          }

          // Start heartbeat BEFORE saving the new token
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
          // 走统一的登出清理路径：失败点可能出现在后半程（startHeartbeat 或 saveConfig
          // 抛错），此时心跳线程已经起来了；只清 token 会把心跳留在后台继续续期旧会话，
          // 而界面已经回到登录页，用户再登录一次就会起第二个。
          // _doLogout 会停心跳、清 token、回填账号密码并带着提示回到登录页。
          await _doLogout({ notice: I18n.t('login.autoLoginFailed') + ': ' + autoError });
          return;
        }

        // Success: go to panel (hardware info will be fetched locally)
        switchView('panel', {
          serverUrl: cfg.server_url,
          fingerprint: fp,
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
    // 已保存的账号密码默认回填，用户不必重新输入；没有配置文件时 cfg 为 null，表单留空。
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

  var LOGIN_MIN_HEIGHT = 320;  // 登录页窗口基准高度，内容放不下时按实际内容撑高

  // 调试构建的标记：标题栏徽标 + 窗口标题前缀。构建类型只有 Rust 侧能判定
  // （cfg!(debug_assertions)），前端拿不到，所以启动时问一次后端。
  async function detectDevBuild() {
    var dev = false;
    try { dev = await API.isDevBuild(); } catch (_) {}
    if (!dev) return;
    _isDev = true;
    document.querySelectorAll('.term-title').forEach(function (el) {
      if (el.querySelector('.term-badge')) return;
      var badge = document.createElement('span');
      badge.className = 'term-badge';
      badge.textContent = 'DEV';
      el.appendChild(badge);
    });
    applyTranslations();
  }

  // 主窗口以 visible: false 创建（见 tauri.conf.json），首屏就绪后由这里显示。
  // 只认第一次：后续 switchView 还会调它，但那时窗口早已可见。
  function revealWindow() {
    if (_revealed) return;
    _revealed = true;
    API.showWindow().catch(function () {});
  }

  // 登录页内容高度 = 标题栏 + 正文 + shell 上下边框。量各部件而不是 shell 自身，
  // 因为 shell 是 height:100%，量出来恒等于窗口高度，反映不了内容有没有溢出。
  function measureLoginHeight() {
    var shell = document.querySelector('#login-view .terminal-shell');
    if (!shell) return LOGIN_MIN_HEIGHT;
    var titlebar = shell.querySelector('.term-titlebar');
    var body = shell.querySelector('.term-body');
    // 上下边框高度与窗口高度无关，由 offset/client 之差直接得到，免得写死数值
    var borderY = shell.offsetHeight - shell.clientHeight;
    var h = (titlebar ? titlebar.offsetHeight : 0) + (body ? body.offsetHeight : 0) + borderY;
    return Math.max(LOGIN_MIN_HEIGHT, Math.ceil(h));
  }

  // 登录页窗口高度贴合内容：出错时下方会多一行提示，固定 320 会把它裁掉。
  function fitLoginWindow(onDone) {
    var done = onDone || function () {};
    if (!document.querySelector('#login-view .terminal-shell')) { done(); return; }
    // 等布局完成，否则量到的是上一帧的尺寸
    requestAnimationFrame(function () {
      try {
        API.resizeWindow(360, measureLoginHeight()).then(done, done);
      } catch (_) { done(); }
    });
  }

  function applyTranslations() {
    document.title = (_isDev ? '[DEV] ' : '') + I18n.t('login.title');
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
    // 文案长度随语言变化，正在显示登录页时窗口高度要重新贴合
    if (document.getElementById('login-view').classList.contains('active')) {
      fitLoginWindow();
    }
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
      // 尺寸定下来再放窗口出来，否则首帧会以创建时的尺寸闪一下
      API.resizeWindow(360, 624).then(revealWindow, revealWindow);
      Background.stop();
      Panel.show(state);
    } else if (name === 'auto-login') {
      loginView.classList.remove('active');
      panelView.classList.remove('active');
      html.classList.remove('panel-active', 'login-active');
      html.classList.add('auto-login-active');
      autoLoginView.classList.add('active');
      if (gearBtn) gearBtn.style.display = 'none';
      API.resizeWindow(360, 320).then(revealWindow, revealWindow);
      Background.stop();
    } else if (name === 'login') {
      panelView.classList.remove('active');
      autoLoginView.classList.remove('active');
      html.classList.remove('panel-active', 'auto-login-active');
      html.classList.add('login-active');
      loginView.classList.add('active');
      if (gearBtn) gearBtn.style.display = 'flex';
      // 高度按内容贴合（出错时会多一行提示），尺寸定下来再放窗口出来
      fitLoginWindow(revealWindow);
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

  // 登出会停心跳、写 config、切视图。这些调用可能并发到来（设备被撤销、连接丢失、
  // 用户手动退出、自动登录失败），并行执行时两条流程会交叉写配置、切换视图，用户最终停在哪一页
  // 取决于谁后跑完。这里把每次调用排队串行执行 —— 不合并，因为各自的提示语不同。
  var _logoutQueue = Promise.resolve();

  // @param options.notice       显示在登录页上，说明这次为什么被退回登录
  // @param options.dropPassword 一并丢弃已保存的密码（只给「刚改过密码」用，见下）
  function _doLogout(options) {
    var opts = options || {};
    var run = function () { return doLogout(opts); };
    // 无论前一次成功还是失败都继续排队，一次登出失败不该让后续登出永远排不上。
    var queued = _logoutQueue.then(run, run);
    _logoutQueue = queued.then(function () {}, function () {});
    return queued;
  }

  // Cleanup: stop heartbeat, clear token, return to the login form.
  //
  // 账号密码一律按已保存的值回填：手动退出、心跳中断、设备被撤销，都只是「这次会话结束了」，
  // 没理由让人每次都重新敲一遍。唯一例外是刚改过密码（options.dropPassword）—— 盘上留的是
  // 已经失效的旧密码，回填它只会让下一次登录必然报「用户名或密码错误」。
  async function doLogout(options) {
    var opts = options || {};
    try { await API.stopHeartbeat(); } catch (e) { console.error('stopHeartbeat failed:', e); }
    Panel.cleanup();
    var cfg = await API.loadConfig();
    var savedUsername = '';
    var savedPassword = '';
    if (cfg) {
      if (cfg.server_url && cfg.token) {
        try { await API.serverLogout(cfg.server_url, cfg.token); } catch (e) { console.error('serverLogout failed:', e); }
      }
      if (opts.dropPassword) {
        // 置空后 save_config 会连凭据库里的条目一起删掉（见 config.rs 的 offload_secret），
        // 否则下次 load_config 又会把旧密码填回来，等于没清。
        cfg.password = '';
      }
      savedUsername = cfg.username || '';
      savedPassword = cfg.password || '';
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
    switchView('login', { username: savedUsername, password: savedPassword, error: opts.notice });
  }

  // `options` 见 _doLogout。
  async function logout(options) {
    await _doLogout(options);
  }

  async function quitApp() {
    try { await API.stopHeartbeat(); } catch (e) { console.error('quit: stopHeartbeat failed:', e); }
    Panel.cleanup();
    API.quit();
  }

  // 会话已失效但用户没主动退出时使用（如设置页改了服务器地址）：停服务、清 token、
  // 回登录页并回显账号密码。
  function invalidateSession(notice) {
    return _doLogout({ notice: notice });
  }

  return {
    init: init,
    switchView: switchView,
    applyTranslations: applyTranslations,
    fitLoginWindow: fitLoginWindow,
    revealWindow: revealWindow,
    logout: logout,
    quitApp: quitApp,
    invalidateSession: invalidateSession,
    checkUpdate: checkUpdate,
    installUpdate: installUpdate
  };
})();

document.addEventListener('DOMContentLoaded', function () {
  App.init();
  // 窗口是 visible:false，正常路径由 switchView 显示。万一启动流程卡住（配置解密失败、
  // IPC 异常），这里兜底把窗口亮出来，不能留用户面对一个永远不出现的应用。
  setTimeout(function () { App.revealWindow(); }, 2500);
});
