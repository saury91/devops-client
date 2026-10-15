// Settings panel — server URL, language, connection test.
var Settings = (function () {
  'use strict';

  var _visible = false;
  var _langSelect;
  // 启动与打开：两项互相约束，值在内存里维护，保存时写回 config
  var _afterLogin = 'panel';   // 'panel' | 'workbench'
  var _openMode = 'inapp';     // 'inapp' | 'browser'

  function init() {
    document.getElementById('gear-btn').addEventListener('click', toggle);

    document.getElementById('settings-close').addEventListener('click', hide);
    document.getElementById('settings-save').addEventListener('click', save);
    _langSelect = document.getElementById('lang-select');

    initSegGroup('seg-after-login', function (val) { _afterLogin = val; });
    initSegGroup('seg-open-mode', function (val) { _openMode = val; });

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

  // --- 分段选择（登录后落点 / 打开方式） ---

  /**
   * 给一个 .seg-group 绑点击：点哪项哪项生效，然后统一交给 syncLaunchOptions() 收口。
   *
   * @param id       容器 id
   * @param onChange 选中值变化时的回调
   */
  function initSegGroup(id, onChange) {
    var group = document.getElementById(id);
    if (!group) return;
    group.addEventListener('click', function (e) {
      var btn = e.target.closest('.seg-btn');
      if (!btn || btn.classList.contains('is-disabled')) return;
      onChange(btn.dataset.val);
      syncLaunchOptions();
    });
  }

  /**
   * 两项互斥约束 + 渲染选中态。
   *
   * 规则（见 docs/workbench-in-app.md 5.1）：
   *   · 登录后 = 直接打开工作台 → 打开方式必须内置窗口，否则登录后会被甩到外部浏览器
   *   · 打开方式 = 系统浏览器   → 登录后必须停在状态面板，同上
   * 不成立的组合直接置灰，而不是允许配出来再解释。
   */
  function syncLaunchOptions() {
    var afterLogin = document.getElementById('seg-after-login');
    var openMode = document.getElementById('seg-open-mode');

    function paint(group, current) {
      if (!group) return;
      Array.prototype.forEach.call(group.querySelectorAll('.seg-btn'), function (btn) {
        var val = btn.dataset.val;
        btn.classList.toggle('is-active', val === current);
        // 该选项是否被另一项锁死
        var locked = (group === afterLogin)
          ? (val === 'workbench' && _openMode === 'browser')
          : (val === 'browser' && _afterLogin === 'workbench');
        btn.classList.toggle('is-disabled', locked);
      });
    }

    paint(afterLogin, _afterLogin);
    paint(openMode, _openMode);

    var hint = document.getElementById('seg-hint');
    if (hint) {
      hint.textContent = (_afterLogin === 'workbench' || _openMode === 'browser')
        ? I18n.t('settings.launchHintLinked') : '';
    }
  }

  function show() {
    _visible = true;

    var urlInput = document.getElementById('settings-url');
    API.loadConfig().then(function (cfg) {
      urlInput.value = (cfg && cfg.server_url) ? cfg.server_url : '';
      // 落点配置读进来：空值按默认（停在面板 / 内置窗口）
      _afterLogin = (cfg && cfg.after_login === 'workbench') ? 'workbench' : 'panel';
      _openMode = (cfg && cfg.open_mode === 'browser') ? 'browser' : 'inapp';
      syncLaunchOptions();

      // 未登录时整组隐藏：还没有面板可落，"登录后去哪"无从谈起。
      // 顺带解决一个硬约束 —— 登录页窗口只有约 400px 高，多这两行会把保存按钮顶出窗口
      var launchGroup = document.getElementById('launch-group');
      if (launchGroup) {
        launchGroup.style.display = (cfg && cfg.token) ? '' : 'none';
      }
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
        cleared.after_login = _afterLogin;
        cleared.open_mode = _openMode;
        await API.saveConfig(cleared);
        return;
      }

      cfg.server_url = url;
      cfg.language = lang;
      cfg.after_login = _afterLogin;
      cfg.open_mode = _openMode;
      await API.saveConfig(cfg);
      hide();

      var btn = document.getElementById('settings-save');
      var label = btn.querySelector('.btn-text');
      var orig = label.textContent;
      label.textContent = I18n.t('settings.saved');
      // 主按钮现在是青色实底，靠改文字颜色反馈会直接糊在底色上，改用视觉类
      btn.classList.add('is-done');
      setTimeout(function () {
        label.textContent = orig;
        btn.classList.remove('is-done');
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
