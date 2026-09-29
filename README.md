# DevOps Client

基于 Tauri v2 的桌面设备认证代理。客户端负责设备指纹绑定、工作台一次性票据签发与心跳保活，为“仅在当前机器可用”提供本机侧的依据。

## 客户端流程

```
登录 / 自动登录
  → 加密保存配置（serverUrl、token、loginAt 等）
  → 启动 30 秒心跳
  → 进入已连接面板
```

点击面板“工作台”圆球时：

```
申请一次性 exchange-token（服务端同时下发工作台设备证明的 challenge 与 nonce）
  → 交给系统默认浏览器打开工作台
  → 浏览器用不可导出的 WebCrypto 密钥对登记并持续签名心跳
```

客户端不再需要监听本地端口，在线状态由心跳维持。工作台的设备证明由浏览器侧完成：
私钥只存在浏览器本地，Session Cookie 被拷到其它机器后签不出有效签名，会话会在证明窗口到期后失效。

心跳按收到的状态执行不同动作：

- `Active` / `Pending`：失败计数清零
- `Revoked`：提示“设备已被撤销”并退出应用
- `SESSION_INVALID`：用设备指纹静默重建会话（正常情况下用户无感）
- `FINGERPRINT_MISMATCH`：安全事件（该会话不属于本机），不做免密重建、不再重试心跳，提示重新登录
- 其他错误/网络异常：累计失败，达到 3 次后返回登录页

---

## 功能特性

- **设备绑定** — ED25519 + SHA256 生成设备指纹
- **设备审批** — 新设备首次登录展示当前审批状态，未通过时在面板给出提示
- **安全打开工作台** — 通过一次性 exchange-token 兑换浏览器 session，并下发设备证明引导数据
- **客户端心跳** — 自动续期、撤销检测、三次失败回登录页（同时维持工作台的设备在场状态）
- **凭据加密** — 密码与会话 token 随整份 `config.json` 一起 AES-256-GCM 加密落盘，不依赖系统凭据库
- **应用自动更新** — 启动时检查并在界面提示，下载与安装由用户在设置页确认
- **国际化** — 中/英双语，根据系统 locale 自动检测
- **系统托盘** — 关闭窗口后常驻后台，托盘菜单可快速打开/退出
- **跨平台** — macOS（ARM64 / x64）、Windows（x64）、Linux（x64）

---

## 环境要求

| 平台 | 依赖 |
|------|------|
| macOS | Xcode Command Line Tools |
| Linux | `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` |
| Windows | Microsoft Visual Studio C++ Build Tools |
| 全部 | Rust 1.82+，Node.js 20+，pnpm，just |

---

## 快速开始

```bash
# 1. 安装依赖
just install

# 2. 开发模式（热重载）
just dev

# 3. 仅类型检查
just check

# 4. 代码格式化与 lint
just fmt
just lint

# 5. 本地构建 macOS ARM64 DMG
just build
```

构建产物：

```
src-tauri/target/aarch64-apple-darwin/release/bundle/
├── macos/DevOps Client.app
└── dmg/DevOps Client_<version>_aarch64.dmg
```

---

## 项目结构

```
src/                            # 前端（WebView UI）
├── index.html                  # Shell 页面（data-i18n 属性）
├── css/styles.css              # 深色主题
├── fonts/                      # Inter / JetBrains Mono 本地字体
├── js/
│   ├── app.js                  # 入口、视图切换、启动/退出、更新检查与安装
│   ├── api.js                  # Tauri IPC 封装
│   ├── i18n.js                 # 前端 i18n 引擎
│   ├── avatar.js               # 默认头像 SVG 生成
│   └── views/
│       ├── login.js            # 登录表单逻辑
│       ├── panel.js            # 已连接面板（用户信息、日志、工作台入口）
│       ├── background.js       # 动态背景
│       ├── wave.js             # 监视器圆球水波纹动画
│       └── settings.js         # 设置页
└── locales/
    ├── en.json                 # 英文翻译
    └── zh.json                 # 中文翻译

src-tauri/                      # Tauri Rust 后端
├── Cargo.toml
├── tauri.conf.json             # 窗口、托盘、CSP、打包目标
└── src/
    ├── main.rs                 # 入口：panic hook、窗口、托盘、生命周期
    ├── lib.rs                  # 模块声明
    ├── commands.rs             # 全部 Tauri IPC 命令
    ├── state.rs                # HeartbeatState
    ├── config.rs               # Config 加载/保存 + 错误日志与 panic 落盘
    ├── fingerprint.rs          # ED25519 + SHA-256 设备指纹
    ├── crypto.rs               # AES-256-GCM 本地加密
    ├── auth.rs                 # Reqwest HTTP 客户端（登录/心跳/换 token/用户信息）
    ├── platform.rs             # 平台相关工具
    └── i18n.rs                 # 语言检测 + 翻译表
```

---

## 配置与数据

所有数据存储在 `~/.devops-client/`：

```
.devops-client/
├── device.key          # ED25519 密钥对（JSON: seed + pub）
├── config.json         # 加密缓存：serverUrl、fingerprint、loginAt 等
└── error.log           # 错误与 panic 记录（超过 1 MiB 滚动为 error.log.1）
```

`config.json` 使用 AES-256-GCM 加密存储，不是明文 JSON，请勿手动编辑。首次写入时由 `crypto.rs` 根据机器 UUID 与用户名派生密钥。

### 密码与会话 token 的存放位置

**账号密码与会话 token 都在这份文件里**，随整份文件一起 AES-256-GCM 加密，密钥由机器 UUID 与登录用户名派生。

早前的版本把它们外移到系统凭据库（macOS 钥匙串 / Windows 凭据管理器），代价是几乎每次重新构建客户端都要弹一次钥匙串授权框 —— 开发构建的二进制一变签名，macOS 就把它当成陌生程序重新要权限，把场景验证与自动化都挡在门外。这条链路已整体移除：`secret.rs` 删除、`keyring` 依赖删除、文件里也不再有需要回填的空字段。

需要自行承担的残余风险：同机同用户的其它进程可以推导出同一个密钥，从而读出 `config.json` 里的密码。这是拿本机隔离性换可用性的取舍；若某天要把这道门槛修回来，正确做法是把秘密重新交给系统凭据库，而不是在文件里做「混淆」。

- 退出登录只清 `token`，`password` 保留，回到登录页时自动回填，不必重新输入。
- 在面板里改过密码后自动退回登录页时 `password` 也会被清掉 —— 那时留着的是已经失效的旧密码，回填它只会让人登录失败。
- 从凭据库版本升级上来的安装，文件里的 `token` / `password` 是空的，需要重新登录一次；登录后写入的就是完整配置。

### 日志

`error.log` 记录配置读写失败，以及**所有 panic（含文件与行号）**。Windows 上没有控制台，主线程之外的 panic 不会显示在任何地方，这份日志是唯一线索。单个文件上限 1 MiB，超过后滚动为 `error.log.1`（只保留一代），避免心跳持续失败把用户目录写满。

面板的「导出」按钮会把界面上看到的事件日志与 `error.log` 的尾部一起写入文件，排查时直接附上即可。

---

## IPC 命令

| 命令 | 说明 |
|------|------|
| `get_lang` | 检测系统语言 |
| `get_fingerprint` | 获取设备指纹 |
| `load_config_cmd` | 加载本地缓存配置 |
| `save_config_cmd` | 保存配置到本地 |
| `get_hostname` | 获取 OS 主机名 |
| `get_os_info` | 获取 OS、OS 版本、客户端版本 |
| `get_device_info` | 汇总设备信息供面板展示 |
| `do_login` | 账号密码登录并保存会话 |
| `auto_login` | 用设备指纹免密自动登录 |
| `get_user_info` | 获取当前登录用户信息 |
| `change_password` | 修改账号密码 |
| `server_logout` | 退出登录并让本地会话失效 |
| `test_connection` | 探测所配置地址的可达性与延迟 |
| `open_browser` | 使用系统默认浏览器打开 URL |
| `open_dashboard` | 换取一次性凭证并打开工作台 |
| `get_dashboard_url` | 只构造工作台地址，不打开浏览器 |
| `start_heartbeat` | 启动 30 秒心跳循环 |
| `stop_heartbeat` | 停止心跳循环 |
| `resize_window` | 调整窗口大小 |
| `minimize_window` | 最小化窗口 |
| `hide_window` | 隐藏窗口到托盘 |
| `quit_app` | 退出应用 |
| `start_drag` | 开始窗口拖拽 |
| `export_log_file` | 把日志内容写入用户选择的路径 |
| `read_error_log` | 读取 `error.log` 尾部（供导出日志附带） |
| `export_device_key` | 导出 `device.key`（Base64） |
| `import_device_key` | 导入备份的 `device.key`，校验后再落盘 |

---

## 自动更新

更新元数据来自 GitHub Releases 的 `latest.json`，安装包签名用 `tauri.conf.json` 中 `plugins.updater.pubkey` 校验。

流程刻意拆成“检查”与“安装”两步：

1. **启动时检查（自动）** — 只提示新版本，不下载、不安装。静默替换二进制并重启会打断正在进行的工作。
2. **设置页 → 应用更新** — 显示当前版本；「检查更新」明确回答“已是最新 / 发现新版本 / 检查失败”；确认后「立即更新」才下载安装，过程中在按钮旁显示进度（`Started` / `Progress` / `Finished` 三段回调），完成后自动重启。

---

## 国际化

系统语言从环境变量（`LANG`、`AppleLocale`）自动检测，默认回退中文。

| 语言 | 代码 |
|------|------|
| English | `en` |
| 中文 | `zh` |

新增语言：
1. 创建 `src/locales/{code}.json`
2. 在 `src-tauri/src/i18n.rs` 中添加 key
3. 更新 `detect_lang()` 中的检测逻辑

---

## 平台支持

| 平台 | 本地开发 | CI |
|------|----------|----|
| macOS ARM64 | `just build` | ✓ |
| macOS x86_64 | `just build-mac-x64` | ✓ |
| Windows x86_64 | — | ✓ |
| Linux x86_64 | — | ✓ |

`tauri.conf.json` 中 `bundle.targets` 已配置为 `"all"`，各平台 CI 会自动构建当前平台支持的所有安装包格式。

---

## 测试

```bash
just test   # cargo test
just ci     # check + fmt + test + lint，与 CI 完全一致
```

CI（`.github/workflows/build.yml`）在 macOS / Ubuntu / Windows 三平台上执行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`，以及前端 `node --check`。

提交前请至少跑一次 `just ci`。`just lint` 与 CI 的 clippy 范围保持一致（都带 `--all-targets`）—— 少了它就不会检查测试代码，会变成“本地全绿、CI 报错”。

Rust 侧的 HTTP 契约测试用 axum 起真实桩服务（随机端口）并通过真实请求打过去，覆盖 URL 拼接、请求头名、JSON 字段名（camelCase）与错误码映射 —— 这些恰恰最容易写错，而坏了在界面上只表现为一句“登录失败”。

---

## 常见问题

### macOS 更新图标后仍显示旧图标

macOS 会缓存应用图标。替换 `src-tauri/icons/` 并重新构建后，若 Dock/启动台仍显示旧图标，可执行：

```bash
# 1. 移除图标缓存
rm -rf /private/var/folders/*/*/*/com.apple.dock.iconcache
rm -rf /private/var/folders/*/*/*/com.apple.iconservices.store

# 2. 重置 Dock 与 Finder
killall Dock
killall Finder
```

### 升级后需要重新登录一次

从使用系统凭据库的版本升上来时，`config.json` 里的 `token` 与 `password` 是空的（秘密当时存在钥匙串里，现在已经不再读取），因此第一次启动会回到登录页、密码框也是空的。登录一次即恢复正常，此后凭证与其它字段一起写在加密的 `config.json` 里。

另外：`config.json` 现在含密码，备份设备身份请使用面板的「导出设备密钥」，不要备份这个文件。

### 应用启动后行为异常（Windows）

Windows 上没有控制台，主线程之外的 panic 不会显示。请查看或导出 `~/.devops-client/error.log`（面板「导出」按钮会一并带上），其中记录了 panic 的消息与发生位置。

---

## 开发规范

- Rust 代码遵循 `cargo fmt` 与 `cargo clippy --all-targets -- -D warnings`（与 CI 一致）
- `Cargo.toml` 的 `rust-version` 是 `clippy::incompatible_msrv` 的输入，调低它会让 CI 立刻报出代码里更高版本 API 的用法
- 前端无打包工具，所有 JS 模块通过全局变量暴露
- 新增 IPC 命令需在 `commands.rs` 实现、在 `main.rs` 的 `generate_handler!` 中注册，并在前端 `api.js` 封装
- 新增界面文案需同时补充 `src/locales/en.json` 与 `zh.json`，两边 key 必须对齐
- 不要在前端或 Rust 中硬编码服务器地址、密钥等敏感信息

---

## License

MIT，见 [LICENSE](LICENSE)。
