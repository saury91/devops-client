# CLAUDE.md

DevOps Client — Tauri v2 桌面设备认证代理。作为 Web 后端服务的配套客户端，提供设备指纹注册、客户端心跳、安全打开 Web 工作台（含设备证明引导票据）、系统托盘常驻和国际化（中/英）能力。

---

## 技术栈

| 层 | 技术 |
|---|------|
| 壳 | Tauri v2 (Rust) |
| UI | 纯 HTML/CSS/JS（WebView 渲染，无打包工具） |
| HTTP 服务 | Axum（本地 HTTP） |
| HTTP 客户端 | Reqwest |
| 设备指纹 | ed25519-dalek + SHA-256 |
| 本地加密 | AES-256-GCM（`crypto.rs`，用于凭据本地保护） |
| i18n | 自研（LazyLock + HashMap / JSON） |
| 打包 | Tauri Bundler → DMG / MSI / AppImage / DEB |

---

## 项目结构

```
src/                          # 前端（WebView UI）
├── index.html                # Shell 页面（data-i18n 属性）
├── css/styles.css            # 深色主题样式
├── fonts/                    # 本地字体（Inter / JetBrains Mono）
├── js/
│   ├── app.js                # 入口：视图切换、启动流程、退出逻辑
│   ├── api.js                # Tauri IPC 封装
│   ├── i18n.js               # 前端 i18n 引擎
│   ├── avatar.js             # 默认头像 SVG 生成
│   └── views/
│       ├── login.js          # 登录表单逻辑
│       ├── panel.js          # 已连接面板：用户信息、日志、工作台入口
│       ├── background.js     # 动态背景（网格/粒子）
│       ├── wave.js           # 监视器圆球水波纹动画
│       └── settings.js       # 设置页（预留）
└── locales/
    ├── en.json               # 英文翻译
    └── zh.json               # 中文翻译

src-tauri/                    # Tauri Rust 后端
├── Cargo.toml
├── tauri.conf.json           # 窗口、托盘、CSP、打包目标
└── src/
    ├── main.rs               # 入口：窗口、托盘、生命周期、全局快捷键
    ├── lib.rs                # 模块声明
    ├── commands.rs           # 全部 Tauri IPC 命令
    ├── state.rs              # HeartbeatState
    ├── config.rs             # Config 结构体 + 读写（~/.devops-client/config.json，AES 加密）
    ├── fingerprint.rs        # ED25519 + SHA256 设备指纹
    ├── crypto.rs             # AES-256-GCM 本地加密/解密
    ├── auth.rs               # Reqwest HTTP 客户端 + 设备认证 API
    ├── error.rs              # AppError（带 i18n 消息）
    └── i18n.rs               # 语言检测 + 翻译表
```

---

## 命令

使用 [just](https://github.com/casey/just) 执行任务（见 `justfile`）：

```bash
just install    # pnpm install
just dev        # Tauri dev 模式（带热重载）
just debug      # Tauri dev --no-watch（快速启动测试）
just build      # 构建 macOS ARM64 DMG
just build-mac-x64  # 构建 macOS x64
just build-all  # CI：构建所有平台
just check      # cargo check（仅类型检查）
just fmt        # cargo fmt
just lint       # cargo clippy -- -D warnings
just clean      # 清理构建产物
just verify     # check + build
just run        # 打开构建后的 App
```

`just dev`（`tauri dev`）下 app 进程归 tauri-cli 管理：手动 kill 掉 app 会被重新拉起 —— 只要 watcher 认为有变化
（保存任何文件、改配置、Rust 重编），CLI 就会杀掉旧进程再 spawn 一个新的，看起来像"杀不死"。想手动杀 app 做冷启动
测试，用 `just debug`（`--no-watch`）；要停掉整条 dev 链，回到跑 `just dev` 的终端 Ctrl+C。

---

## 架构

### 模块职责

| 模块 | 职责 |
|------|------|
| `main.rs` | 入口：窗口创建、系统托盘、生命周期事件、窗口拖拽/关闭行为 |
| `commands.rs` | 所有 `#[tauri::command]` IPC 处理器 |
| `state.rs` | 状态管理：心跳状态、当前 fingerprint |
| `config.rs` | 配置读写：`~/.devops-client/config.json`（AES-256-GCM 加密） |
| `fingerprint.rs` | 设备指纹：ED25519 密钥对 + SHA256 |
| `crypto.rs` | 本地 AES-256-GCM 加密，密钥由机器 UUID + 用户名派生 |
| `auth.rs` | API 客户端：`login-device`、`auto-login`、`device-status`、`create-exchange-token`、`get-user-info` |
| `error.rs` | 错误类型：带 i18n 消息的 AppError |
| `i18n.rs` | 国际化：Lang 枚举、语言检测、翻译函数 |

### 启动流程

```
main()
  → 生成/读取设备指纹
  → 检测系统语言
  → 创建主窗口（360×320，深色主题，无边框可拖拽）
  → 配置系统托盘（左键显示、菜单打开/退出）
  → 注册 IPC 命令
  → 前端加载 config.json，若有缓存 token 则尝试自动登录
```

### 登录流程

```
UI 表单 → API.doLogin(url, user, pass, deviceName)
  → Rust POST /api/auth/login-device（携带 fingerprint、os、clientVersion、deviceInfo）
  → 成功:
    → 保存 config.json（serverUrl、token、fingerprint、loginAt 等，AES 加密）
    → 启动心跳（每 30s POST /api/auth/device-status，Header X-Session-Id）
    → 切换到已连接面板
  → pending: 显示"需要管理员审批"
  → error: 显示服务端错误信息
```

### 打开工作台

```
点击面板"工作台"圆球
  → Rust POST /api/auth/create-exchange-token（X-Session-Id）
  → 获取 exchangeToken
  → 系统浏览器打开 /api/auth/exchange-token?exchangeToken=...
  → 后端校验原 session → 新建浏览器独立 session → 写入 SESSION_ID cookie
  → 按账号类型重定向：普通账号 → /dashboard，管理账号 → /index
```

点击期间设置 `_openingBrowser` 防抖，防止重复打开。

### 心跳与离线策略

- 心跳每 30 秒调用 `/api/auth/device-status`，携带 `X-Session-Id` 和 `fingerprint`（间隔常量 `HEARTBEAT_INTERVAL_SECS`）
- 服务端校验 fingerprint 与 session 是否匹配：
  - 匹配：renewal 当前 session 及同 fingerprint 的所有 browser session
  - 不匹配：立即失效当前 session，并踢掉 fingerprint 所属用户的所有 session
- 客户端状态处理：
  - `Active` / `Pending`：失败计数清零
  - `Revoked`：发送 `device-revoked` 事件并退出应用
  - `SESSION_INVALID`：先用指纹静默重建会话（`renew_session`），成功发 `session-renewed` 并清零失败计数；重建失败发 `connection-lost`
  - `FINGERPRINT_MISMATCH`：安全事件（该会话不属于本机），不做免密重建、不再重试心跳；写 `error.log` 并发送 `device-identity-mismatch`
  - 其他 `Error` / `NotFound` 或网络错误：累计失败，达到 3 次后发送 `connection-lost`
- 前端收到 `connection-lost` / `device-identity-mismatch` 后调用 `_doLogout()`：停止心跳、清除 token、回填账号密码并返回登录页

### 安全模型

1. 用户通过密码登录 → 服务端创建 session，客户端保存 token
2. 打开工作台时签发一次性 exchange-token → 服务端据此建立浏览器会话，并随响应下发
   工作台设备证明的 challenge 与 nonce
3. 浏览器用不可导出的 WebCrypto 密钥对登记公钥并持续签名心跳 →
   服务端刷新「已证明」标记，`AuthInterceptor` 每请求只读这个标记
4. 心跳持续上报 → 刷新设备在场状态，设备被撤销或服务端 session 失效时客户端立即退出
5. Cookie 拷到其他机器 → 缺少匹配的私钥（签不出有效签名）或设备不在场 → 服务端拦截请求并失效对应 session
6. fingerprint 与 session 不匹配 → 双杀：当前 session 失效 + fingerprint 所属用户全部 session 失效

### 指纹算法

```
raw = systemUUID + "/" + hostname + "/" + base64url(ed25519_pubkey)
fingerprint = SHA256(raw) → 64 位小写 hex
```

密钥持久化在 `~/.devops-client/device.key`（JSON: `{seed, pub}`）。

---

## 数据与配置

所有本地数据位于 `~/.devops-client/`：

```
.devops-client/
├── device.key          # ED25519 设备密钥（明文 JSON）
└── config.json         # AES-256-GCM 加密缓存（非明文，请勿手动编辑）
```

- `config.json` 使用 AES-256-GCM 加密，密钥由机器 UUID + 当前用户名派生，换机器无法解密。
- 如需迁移配置，必须重新登录。

---

## i18n

### 添加语言

1. 创建 `src/locales/{code}.json`（key 与 en.json 一致）
2. 在 `i18n.rs` 的 `i18n_map!` 宏中添加对应翻译
3. 在 `i18n.rs` 的 `detect_lang()` 中添加语言检测逻辑

### 前端用法

```html
<span data-i18n="login.title">Fallback text</span>
```

JS: `I18n.t('login.title')`

### Rust 用法

```rust
let msg = i18n::t(lang, "login.failed");
```

---

## 打包与跨平台

- `tauri.conf.json` 中 `bundle.targets` 使用 `"all"`，CI 各平台会自动构建该平台支持的所有安装包。
- macOS 本地构建：`just build`（ARM64）、`just build-mac-x64`（x64）。
- Windows / Linux 构建需在对应平台（或 CI）运行。

### 图标资产

`src-tauri/icons/` 下所有 PNG/ICNS/ICO **都是生成物，不要手改**。唯一源文件是
`src-tauri/icons/app-icon.svg` 与 `tray-template.svg`，由 `scripts/gen-icons.py` 生成：

```bash
just icons          # = python3 scripts/gen-icons.py
```

脚本负责：写 SVG 与 `icon-manifest.json` → 调 `tauri icon` 栅格化全平台图标 → 单独渲染托盘 PNG。

几条必须知道的约束：

- **标记语义**：云（数据云 / 云管平台）+ 环绕轨道环 + 掠过的光带 + 一串下落的 `0/1`
  （上下行数据流）。改图时不要丢掉这个语义组合
- **别做"云 + 竖直等长线条"**：那是"下雨"图标，语义会跑偏。数据位必须与云体分离、有明暗渐变
- **`0/1` 画成矢量字形、不依赖字体**：resvg 栅格化时找不到字体会退化成方块；且必须**保持直立**，
  顺着环的切线旋转会让环底部的数字上下颠倒
- **数据位与光带并行、不相交**：压在光带上会被切断，两层信息互相干扰
- **云体暗部不能压到背景明度**：曾出现轮廓多处对比差 < 10（右侧穹顶只有 1.2），Dock 里整朵云糊成一片。
  现在靠两件事保证剪影：① 云体渐变暗部不深于 `#25405F`；② `cloud_edge()` 先把云体并集外扩
  `CLOUD_EDGE` px 铺一层淡蓝白，再叠正常云体 → 外缘留一圈极淡边光。改云体颜色/尺寸后
  要重新量一遍轮廓对比（内侧点 vs 外侧 40px 的背景，亮度差应 > 30）
- **`tauri icon` 不加任何圆角遮罩**（纯栅格化），Apple 的圆角方底必须画进源图。脚本用超椭圆
  （n=5）拟合 Apple 连续圆角，主体 824×824 居中于 1024 画布（Apple 图标网格），Windows/Linux 共用这套几何
- **图标里不放文字**（除 `0/1` 这种图形化数据位）：16px 下文字必然糊成一团
- **托盘图标是 macOS 模板图**（`tray-template.png`：纯黑 + alpha，44px = 22pt@2x），配合 `main.rs` 的
  `icon_as_template(true)` 由系统按明暗主题自动着色。**不要改回硬编码白色**——浅色菜单栏下白色图标会直接消失。
  托盘只保留**云体剪影**：菜单栏图标 16~22pt，环会缩成发丝、`0/1` 会变成脏点
- iOS 图标不允许透明（App Store 会拒），`icon-manifest.json` 的 `bg_color` 负责兜底品牌深色底
- 改了图标后如果 macOS 仍显示旧图，清一次图标缓存（见下）

### macOS 图标缓存

替换 `src-tauri/icons/` 后若 Dock/启动台仍显示旧图标：

```bash
rm -rf /private/var/folders/*/*/*/com.apple.dock.iconcache
rm -rf /private/var/folders/*/*/*/com.apple.iconservices.store
killall Dock
killall Finder
```

---

## 注意事项

- Public repo — 代码中**禁止**包含公司/服务器/内部信息
- 服务器地址由用户手动输入，加密缓存于 `~/.devops-client/config.json`
- 关闭窗口 = 隐藏到托盘，只有托盘菜单"退出"才真正退出
- 主窗口是 `visible: false`，由前端把视图、语言、尺寸都就绪后调用 `show_main_window` 显示（`app.js` 的 `revealWindow`），避免出现 webview 未绘制时的纯色空窗；`app.js` 在 `DOMContentLoaded` 里留有兜底定时器，即使启动流程出异常窗口也不会永远不显示
- **窗口按平台在 Rust 里创建**（`tauri.conf.json` 的 `windows` 是空数组，`main.rs` setup 里分叉），因为窗口形态只能在创建时决定（`transparent` 没有运行时 setter）：
  - **macOS 走 App Store 合规路径**：`decorations(true)` + `TitleBarStyle::Overlay` + 原生标题置空。圆角、阴影、红绿灯全是系统公开 API 给的，**Cargo.toml 不要加 `macos-private-api`**（private API 会上不了 App Store）。Overlay 下系统仍会居中绘制原生标题文字，跟前端自绘标题叠成重影，所以标题必须传空串
  - **Windows 走透明窗口**：`decorations(false)` + `transparent(true)` + `shadow(false)`，圆角由 CSS 画。`shadow` 不能开——DWM 阴影跟矩形框走且附赠 1px 白边
  - 前端用 `html[data-os]` 配合：macOS 下 `--radius-window: 0`、隐藏自绘交通灯和 `#app-shell::after` 贴边描边、**并给 `html` 上不透明底色 `--bg`**；Windows 保留前两样且 html/body 保持透明（圆角靠它）
  - macOS 必须显式设**窗口背景色**（`main.rs` 里 `set_background_color`，即 NSWindow.backgroundColor）。macOS 这条路径窗口是不透明的，改尺寸时新露出的那条边由**窗口服务器**先用窗口背景色填充，而 NSWindow 默认背景是浅色 —— 不设就会每次改尺寸底部闪一下白（登录后切面板那一下最明显，见 2026-10-15 反馈）。**只给 macOS 设**：Windows 下任何非 0 alpha 会被强制成不透明，设了圆角就没了
  - macOS 的标题栏是**两列网格**（自绘灯组 `display:none` 后子元素少一个，沿用三列网格会整体前移一格，标题撞红绿灯区）；`padding-left: 76px` 预留原生灯位，标题左对齐
  - 窗口固定尺寸，macOS 的绿色缩放按钮（zoom）被禁用后会渲染成黑色圆点，`main.rs` 建窗后已用 `standardWindowButton(NSWindowButton::ZoomButton) + setHidden(true)` 隐藏（公开 AppKit API）；自绘交通灯（`.win-actions`）里就**不要**再加关闭/最小化之外的东西
- **窗口是透明的（仅 Windows）**。窗口可见形状由 `#app-shell` 决定（圆角 + 底色 `--bg`），因此 body 必须 `background: transparent`，**任何一处给 html/body 上底色都会让圆角失效**（macOS 例外，见上：macOS 的圆角由系统给，需要不透明底色）
- **工作台是客户端内的第二扇窗**（`commands.rs` 的 `open_workbench` / `open_workbench_url`，设计见 `docs/workbench-in-app.md`）：
  - 加载的是**远端页面，不是我们的代码**，所以**绝不授予 IPC 权限**（`capabilities/default.json` 只挂 `main`）；创建、导航、落点、关闭全部由 Rust 侧驱动
  - 服务端下了 `X-Frame-Options: DENY` + CSP `frame-ancestors 'none'`，**iframe 内嵌走不通**，只能顶层导航
  - 单例：窗口已存在时必须 `navigate` 换地址再聚焦 —— exchange token 是一次性的，旧地址早作废，只 focus 会让用户看到登录页
  - 防白闪三件套：`visible(false)` 创建 + 窗口底色 `#0B1120` + `on_page_load(Finished)` 才 show（3s 兜底）。注意 `on_page_load` **只在 builder 上**，已建好的窗口没有这个方法
  - macOS 用 `Overlay` + `initialization_script` 注入 `.navbar{padding-left:78px}` 让出红绿灯位（**embed 契约前的临时手段**，网页侧支持 `?embed=client` 后删掉）；Windows/Linux 用原生边框
  - 落到 `/login` 或 `/workbench-closed` 时关窗并 emit `workbench-session-ended`；**不静默续期**（用户可能是在网页上主动退出登录，自动登回去会让他退不掉）
- **启动落点有配置**（`config.after_login` = `panel`/`workbench`，`config.open_mode` = `inapp`/`browser`）：两项互相约束，设置页不成立的组合直接置灰（`settings.js` 的 `syncLaunchOptions()`）。触发规则：手输登录 ✓、免密续登 ✓、**从托盘恢复显示 ✗**、将来若加开机自启 ✗（自启用例见文档 5.2）。未登录时这组设置整组隐藏 —— 登录页窗口只有约 400px 高，多两行会把保存按钮顶出窗口
- **登录页的 `.terminal-shell` 必须 `height: auto`，不能 `height: 100%`**。登录页窗口高度是「贴合内容」算出来的（`app.js` 的 `measureLoginHeight`）：shell 一旦撑满窗口，量到的就是当前窗口高度 —— 自我引用，于是登录页被面板的 600 撑过一次后再也回不去（登出后窗口仍是面板高度、内容顶在上面、下方空一大片，见 2026-10-15 反馈）。配套：`.view#login-view` 用 `align-items: flex-start`，且测量只能取 `shell.offsetHeight`（`getBoundingClientRect()` 会被 `.view.active` 入场动画的 transform 缩放）
- 圆角半径走 `--radius-window` 令牌。内层 `.terminal-shell` 必须复用同一个变量——两者是同心关系，半径不一致时内描边会在转角被外壳裁断
- 窗口尺寸常量集中在 `app.js` 顶部（`WINDOW_W` / `PANEL_H` / `SPLASH_H` / `LOGIN_MIN_H`）。面板高度按最矮的 Windows 场景取上限：1366×768 机器在 125% 缩放下逻辑高度约 614，再高会被屏幕切掉底部
- 前端无打包工具，JS 通过全局变量（`API` / `I18n` / `App` / `LoginView` / `Panel` / `Wave` / `Background` / `AvatarUtil`）通信
- 心跳必须在登录成功后启动，登出/连接丢失时务必停止，避免无效请求
- 退出前调用 `/api/auth/device-offline` 通知服务端下线；客户端崩溃/断网时由心跳中断（在场标记超时）兜底
