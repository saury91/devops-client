# macOS 设备证书绑定 — 验证 Checklist

> 本文件唯一目的：**让任何一个结论只被验证一次，之后永不复验。**
> 每验证一项，就地改这一项的状态与证据，不做别的。

对象：`devops-client`（Tauri）在 macOS 上生成并安装客户端证书（`CN=devops-client`）到 login keychain，
由**浏览器**在 TLS 握手时从钥匙串取证书提交给服务端，服务端以 mTLS 完成设备绑定。

状态图例：`TODO` 未验 / `PASS` 通过 / `FAIL` 不通过 / `BLOCKED` 无法自行推进

---

## 0. 铁律（违反即等于重做一遍）

1. **状态一旦落定 PASS/FAIL/BLOCKED，禁止重跑。** 唯一的例外：§1.3 的「装置指纹」变了，此时必须新建一条带指纹的记录，而不是覆盖旧结论。
2. **原始输出必须 append 到证据台账**：`logs/cert-evidence.log`（`*.log` 已被 `.gitignore` 忽略）。
   不允许只依赖 `s_server.err` / `s_server.out` —— 二者每次重启都被覆盖，这正是上一轮"反反复复"的根因。
   每次 append 的格式：
   ```
   ==== <ID> <ISO时间> rig=<s_server PID>@<启动时间> ====
   $ <执行的命令>
   <原样输出>
   ```
3. **一次只推进一个 ID**，跑完立刻回填本表的「状态 + 证据定位」，再动下一个。
4. **浏览器类验证（E/F 段）若自动化不可行，直接标 BLOCKED 交人工点一下，不反复尝试。**
5. 已判定失败的路径见 §6「已烧掉的路径」，**不要再试第二次**。

---

## 1. 验证装置（rig）

### R1–R4 旧装置（`openssl s_server`）的四个致命缺陷 — 已废止，仅在需要理解历史时参考

| ID | 缺陷 | 后果 | 证据 |
|----|------|------|------|
| R1 | stderr 重定向到文件后是**块缓冲**（4096B），握手结果卡在缓冲里，直到进程退出才落盘 | 运行中看日志永远是空的 → **"看起来没验过" → 反复重验**（这才是整场浪费的真正根因） | 当前装置曾有 8 条 ESTABLISHED 连接，`.err` 却只有 1 行；`kill` 后才吐出真相 |
| R2 | `ACCEPT` **不是**按连接计数 | 用 `accept_delta` 判定完全无效（旧装置 5 次以上连接只有 1 行 `ACCEPT`） | `/tmp/fw-tls/s_server.log.old`、`s_server.out` |
| R3 | 不记录对端地址/进程 | 无法区分是 Firefox、Safari、Chrome 还是外部扫描器连的 → 归因靠猜 | `/tmp/fw-tls/round3`、`round4` |
| R4 | `-accept 8443` 绑 `*` 全网卡 | 局域网/扫描器流量混入证据 | `lsof` 显示 `*:8443 LISTEN` |

### R5–R8 新装置（`probe.py` + 独立探针协议）— 现行标准

| ID | 内容 | 状态 | 证据 |
|----|------|------|------|
| R5 | **每轮独立短生命周期探针** + **判定前必须 kill 强制 flush**。禁止在探针存活期间读日志 | PASS | `/tmp/fw-tls/round5.sh`、`round6.sh` |
| R6 | 新仪器 `/tmp/fw-tls/probe.py`：`flush=True` 逐连接记录、**内置 `lsof` 把对端端口翻译成 PID+进程名**、明确区分 `NO-CERT` / `BAD-CHAIN` / `CLIENT-CERT subject_CN=...`、只绑 `127.0.0.1` | PASS | `/tmp/fw-tls/probe.py` |
| R7 | 装置指纹：记录探针 PID + 端口 + 启动时间 + `advertise_ca`，写入台账 | PASS | 台账 `==== ROUND5/ROUND6 ... ====` 行 |
| R8 | 证书实体：`client.pem` 与钥匙串 `devops-client` **指纹完全一致** `766DA371D51BB6ADCDD727EFF4C25BB40BE66934` → 探针下发的 CA 就是钥匙串身份本身 | PASS | `openssl x509 -fingerprint -sha1` vs `security find-certificate -c devops-client -Z` |

### R9 仪器可信度正对照（**没有这一步，前面所有浏览器结论都不算数**）

| ID | 内容 | 状态 | 证据 |
|----|------|------|------|
| R9a | `openssl verify -CAfile client.pem client.pem` → `OK`，证明自签叶证书可作信任锚，探针不会把"正确的证书"误判成 `BAD-CHAIN` | PASS | 14:38 实测 |
| R9b | 系统 TLS 栈（`/tmp/fw-tls/mtls3.swift`，URLSession + 钥匙串身份）提交后，探针输出 `CLIENT-CERT subject_CN=devops-client issuer_CN=devops-client` | PASS | `/tmp/fw-tls/pcontrol2`，14:39 |

> R9b 同时说明：**仪器能正确点亮 `devops-client`**。因此任何浏览器若没被点亮，就是浏览器的问题，不是仪器的问题。

---

## 2. A 段 — 钥匙串实体

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| A1 | 证书与私钥已存在于 **login keychain** | `security find-identity -p ssl-client` 输出含 `devops-client` | PASS | 输出：`2) 766DA371D51BB6ADCDD727EFF4C25BB40BE66934 "devops-client"` |
| A2 | 私钥不可导出（P-256，keychain 内生成） | 代码：`SecKey::new` + `Location::DefaultFileKeychain`，无导出路径 | PASS(代码事实) | `src-tauri/src/cert/macos.rs:126-139` |
| A3 | ACL 与 partition list 均未写入（预期首次必弹钥匙串授权框） | 代码明确不写 ACL | PASS(代码事实, 待运行时佐证见 D2) | `src-tauri/src/cert/macos.rs:17-24` |
| A4 | 证书有效期 397 天 / 提前 30 天续期 | 常量与证书 notAfter 一致 | PASS(代码事实) | `src-tauri/src/cert/mod.rs:36-46` |

---

## 3. B 段 — 信任状态（本轮定位到的根因）

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| B1 | 修复**前**：身份不在 `Valid identities only` 中 | 同一条 `find-identity` 命令下，`devops-client` 只出现在 `Matching identities`，标 `CSSMERR_TP_NOT_TRUSTED` | FAIL(已记录) | 上轮实测输出 |
| B2 | 修复**后**：身份进入 `Valid identities only` | 命令输出 `2 valid identities found` 且含 `devops-client` | PASS | 上轮实测输出 |
| B3 | 修复动作与回退方式可追溯 | 修复：`security add-trusted-cert -r trustRoot -p ssl /tmp/fw-tls/client.pem`（仅用户级、仅 SSL 策略）；回退：`security remove-trusted-cert /tmp/fw-tls/client.pem` | PASS | 上轮执行记录 |

> B 段是 WebKit / Chromium 选择客户端证书所依据的策略。未受信任的身份不会被提供给服务器 —— 这是"浏览器从不弹证书、服务端一直收到空证书"的解释。

---

## 4. C 段 — 非浏览器 TLS 栈（已闭环，**不要再验**）

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| C1 | 系统 TLS 栈（Swift `URLSession` 探针）能提交 `CN=devops-client` | 服务端出现 `depth=0 CN=devops-client` / 探针出现 `CLIENT-CERT subject_CN=devops-client` | **PASS（已用新仪器复现，闭环）** | 旧装置：`/tmp/fw-tls/s_server.log.old`；新仪器：`/tmp/fw-tls/pcontrol2`，`14:39:48 CLIENT-CERT subject_CN=devops-client issuer_CN=devops-client` |
| C2 | 钥匙串中 `devops-client` 是否就是测试用的那张证书 | 指纹一致 | PASS | `766DA371D51BB6ADCDD727EFF4C25BB40BE66934`（`client.pem` == `security find-certificate -c devops-client`） |

> 结论：证书本体与系统 TLS 链路**没有问题**。问题只在"浏览器是否愿意提供该身份"，即 B 段。

---

## 5. D / E / F 段 — 三条浏览器路径

### D 段 Firefox — 结论：**FAIL，可复现，与本项目证书无关**

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| D1 | 修复前 Firefox 是否提交证书 | 出现 `depth=0 CN=devops-client` | FAIL（历史记录，归属系时间推断） | 旧装置 stderr **4 次** `peer did not return a certificate` |
| D2 | 修复信任后 Firefox 复测 | 探针出现 `CLIENT-CERT subject_CN=devops-client` | **FAIL** | `/tmp/fw-tls/round5`：`CONNECT <-- firefox(pid=92205)` 之后 3 次 `CLIENT-CERT subject_CN=www.dingtalkcs.com issuer_CN=GlobalSign RSA OV SSL CA 2018`，1 次 `NO-CERT` |
| D3 | 排除"是我自加的 auto-select pref 造成的" | 删掉该 pref 后结果是否改变 | **已排除，结果不变** | `/tmp/fw-tls/round6`：`user.js` 只剩产品会写的 `security.osclientcerts.autoload`，Firefox 仍在 67ms 内提交 `www.dingtalkcs.com` |
| D4 | 钥匙串里到底有几张身份 | 全量列出 | PASS | `Apple Development: hao wang (D9TRK38UHS)` / `www.dingtalkcs.com` / `devops-client` |
| D5 | 修复假设：`security.default_personal_cert = "Ask Every Time"` 能否让 Firefox 弹选择框、由人选到 `devops-client` | 出现选择框且服务端收到 `devops-client` | **PASS（结案，永不复验）** | `d5/D5c-ASK`（probe_pid=4277 @18753，strict，只下发 `client.pem`）：<br>① 弹框出现且候含有 `devops-client`（人工截图，存于 `OS Client Cert Token`）；<br>② 人工点「确定」后 `14:47:23.636 CLIENT-CERT 127.0.0.1:55781 subject_CN=devops-client issuer_CN=devops-client` |

> **关键判读**：服务端下发的可接受 CA 列表**只有** `CN=devops-client`（`round5` 仪器），Firefox 依旧提交钉钉那张。
> 所以失败点不是"我们的证书不被信任"，而是 **Firefox/NSS 在 `Select Automatically` 下不按服务端 CA 列表收敛候选**。
> 另一条独立证据：正对照（`pcontrol2`）证明同一台仪器能正确点亮 `devops-client` → 不是仪器误判。
>
> **D5c 判定（14:45–14:47 现场，D5 结案）**：装置、探针、`advertise_ca` 全不变（`strict`，只下发 `client.pem`），
> 仅把 pref 换成 `Ask Every Time` → 弹框**出现**，候选里**有** `devops-client`（存储于 `OS Client Cert Token`，
> 即经 osclientcerts 从钥匙串读出）；人工点「确定」后探针出现
> `14:47:23.636 CLIENT-CERT 127.0.0.1:55781 subject_CN=devops-client issuer_CN=devops-client`。
> 这坐实了 D2 的失败点只在"自动选择"这一档，不是证书不被信任。**唯一变量就是该 pref。**
>
> ⚠️ **浏览器报「连接超时」是装置伪影，不是产品缺陷，勿据此改判定**（详见台账 `D5c-ASK 后半`）：
> ① 探针单线程串行握手，`55715` 挂住时 `55781` 只能排队；② Firefox 自身握手上限 ~60s
> （`14:45:22.897 → 14:46:22.260` = 59.4s）；③ 人工在弹框停留超过 60s，Firefox 先超时弃连，
> 证书其后才送达，探针回包时 socket 已关 → `POST-ERROR Broken pipe`。
> 判定标准已满足（弹框出现 + 服务端收到该证书），**按 §0 铁律不再为看一次 HTTP 200 重跑**。

### E 段 Safari / WebKit — 结论：**BLOCKED（需人工一次点击）**

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| E1 | Safari 访问探针 | 探针出现 `CLIENT-CERT` | **BLOCKED** | `/tmp/fw-tls/round4`：`14:35:16 CONNECT ...` 之后再无任何结果行 → 握手挂起在证书选择环节；同期本机存在 7 条 `com.apple.WebKit.Networking`(pid=39944) 的 `ESTABLISHED` 挂起连接 |
| E2 | Safari 是否连得上（**纠正旧结论**） | 出现 `CONNECT` | PASS | 同上；`lsof` 归因 `<-- com.apple.WebKit.Networking(pid=39944)`。此前"Safari 从未发起连接"的判断是**错的**，系旧装置缓冲造成的假象 |

### F 段 Chrome / Chromium — 结论：**PASS（默认配置下弹框一次、人工选中 `devops-client` 即通过，无需任何 pref）**

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| F1 | 关闭抢 profile 的 Playwright Chrome（`--user-data-dir=.../playwright_chromiumdev_profile-ZvA0fN`） | `ps` 中不再有该实例 | **DONE** | 目标 `pid=79490`（父 `79489` = `@playwright/cli` 的 `cliDaemon.js`，Sep 18 常驻；其 helper `79504` 正是 ROUND4 被归因为 "Google 79504" 的污染源）；核对命令串后 `kill 79490`，其与父 daemon 一并退出 |
| F2 | Chrome 访问探针（F1 之后单测） | 探针出现 `CLIENT-CERT subject_CN=devops-client` | **PASS** | `/tmp/fw-tls/f/F2-CHROME`（probe_pid=15827 @18760，strict，只下发 `client.pem`）：`14:51:23.131 CLIENT-CERT 127.0.0.1:56090 subject_CN=devops-client issuer_CN=devops-client`，紧接 `14:51:23.472` 又一次同样结果 |
| F3 | Chrome 是否**弹出**人工选择框（决定 UX 与"零人工"是否可行） | 弹框出现且其中能选到 `devops-client` | **PASS（人工确认）** | 用户确认：「弹出了证书选择对话框，我选了 `devops-client` 后确认」。与时间线完全吻合：`56090` 由 `CONNECT`(17.104) 到 `CLIENT-CERT`(23.131) 隔 **5.03s**（人工选择耗时）；紧随的 `56097` 仅 **1ms**（Chrome 已记住本次决定，不再弹框） |

> ⚠️ **F2 曾有一次判废的尝试，不做重试，只记原因**（避免日后重蹈）：
> `open -a "Google Chrome" URL` 会把请求交给 **`pid=76553`** —— 那是 **Sep 22 09:47 挂死的 headless `--screenshot` 实例**
> （`--user-data-dir=/tmp/chrome-t1`，0 个 socket、无窗口、挂了 6 天），压根不是用户自用的 Chrome。
> 根因：`lsappinfo` 显示本机注册了**三个** `com.google.Chrome` 实例，而 `76553` 是 `type="Foreground"`，
> LaunchServices 优先把 `open` 路由给了它。
> **副作用（与本项目无关，但需知会）**：自 Sep 22 起，`open -a "Google Chrome"` 及点 Dock 图标都会命中这个无窗口僵尸。
> 正确做法（已验证）：绕开 LaunchServices，直接
> `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --new-window <url>`，
> Chrome 的单例机制会输出「正在现有的浏览器会话中打开。」并交回用户真正在用的 `37856`。

> **Chrome 与 Firefox 的关键差异**（同一 strict 口径、同一台仪器、同一张钥匙串身份）：
> `Firefox (Select Automatically)` → 提交 `www.dingtalkcs.com`（错证书）；
> `Chrome` → 提交 `devops-client`（对证书）。
> 推测原因是 **Chromium 会按服务端下发的 CA 列表过滤钥匙串身份**，过滤后仅剩 `devops-client`；
> 而 Firefox/NSS 在 `Select Automatically` 下不做这层收敛（D2/D5 已证）。
> → 产品含义：**Chromium 系无需额外配置即可用；Firefox 必须写 `Ask Every Time`**。

---

## 6. 已烧掉的路径（禁止再试，试了就是浪费）

| 路径 | 结果 | 结论 |
|------|------|------|
| `pkill -f "spring-boot:run"` / 任何**模糊匹配** kill | 被用户拒绝 | **禁用**。只能按 PID 精确杀：`kill <PID>`，先 `ps -eo pid,command \| grep <精确字符串>` |
| AppleScript `set URL of front document` | rc=0 但**不导航**，Safari 标签仍是 `未命名` / `missing value` | 已试 2 次，不要再试 |
| AppleScript `make new document with properties {URL:...}` | 同上 | 已试，不要再试 |
| 用 `openssl s_server` 当仪器 | 绑 `*` 全网卡、stderr 块缓冲、`ACCEPT` 非按连接计数、无对端归因 → 反复产生假空/假负 | **已废止**，换 `/tmp/fw-tls/probe.py` |
| 在探针存活期间读日志做判定 | 缓冲未 flush，读到的是假空 | **禁止**。判定前必须 `kill` 探针强制 flush（R5） |
| 用 `ACCEPT` 增量做判定 | 该字符串不是按连接计数 | **禁止**，改用探针的 `CONNECT`/`CLIENT-CERT` 事件 |
| `curl --cert devops-client` 取钥匙串身份 | 本机 curl 8.7.1(LibreSSL) 把名字当 PEM 文件读，`exit 58` | 不再试；正对照改用 `/tmp/fw-tls/mtls3.swift` |
| 在判定用的探针里 `load_default_certs()` | 会把上百个系统 CA 名写进"可接受 CA 列表"，让钉钉证书变成**合法候选**，从而伪造出"Firefox 选错证书"的假象 | 只在诊断模式用；**判定轮只允许 `advertise_ca=client.pem`** |
| 重建 Firefox profile | 上个 profile 是好的，重建等于把 D1 的证据作废 | 禁止 |
| 用 `open -a Safari` 失败就断言"Safari 不连服务端" | 该结论**是错的**（E2 已纠正）：Safari 连得上，只是握手挂起 | 不要再据此下结论 |

---

## 7. G 段 — 客户端 App 自身（与浏览器正交）

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| G1 | `devops-client --cert-status` 输出与钥匙串一致 | 输出体现证书存在/指纹 | **PASS** | `logs/cert-evidence.log` 的 G1 段（release 二进制，2026-09-28 14:16 构建）：`capability: Full, store: "login keychain", installed: {fingerprint "05d04bb4…0a66", serial "3a476869…f322c9", not_after "2027-10-30T06:17:17Z"}`，与 `security find-certificate -c devops-client` 的 SHA-256 / serial / not_after **逐项一致**，也与 Firefox 弹框显示完全吻合 |
| G2 | `--cert-ensure` 幂等：连续两次不重复导入、不产生第二份密钥 | 第二次输出为已存在/未变更，且 `find-identity` 仍是同一条 SHA-1 | **PASS** | `logs/cert-evidence.log` G2 段：两次 `--cert-ensure` 输出与 BEFORE **逐字相同**（`05d04bb4…` / `3a476869…` / `2027-10-30T06:17:17Z`），`find-identity` 仍为同一条 `766DA371…` 且条数 **1**，全程**无钥匙串弹框** → 命中 `macos.rs:60-62` 的 `status()` 早返回分支，实际**零写入** |
| G3 | `--cert-remove` 后无残留，且再 `--cert-ensure` 可恢复 | 删除后 `find-identity` 无该项；重建后 SHA-1 变化但身份有效 | **PASS（但暴露 G10）** | 删除后：`remove ok`、残留证书计数 **0**、`--cert-status` = `installed: None`；重建后 SHA-1 `766DA371…` → `17737DFC…`，身份重建成功。<br>⚠️ 重建出的身份**是未受信任的**：`find-identity -p ssl-client` 显示 `(CSSMERR_TP_NOT_TRUSTED)`，信任记录从 2 条自动降为 1 条 → **必须人工 `add-trusted-cert` 才可用**（= G10）。恢复动作已执行：信任回 2 条、`2 valid identities` |
| G4 | IPC 命令 `get_cert_status` / `install_device_cert` 已注册 | `main.rs` invoke_handler 含两项 | PASS(代码事实) | `src-tauri/src/main.rs:93-94` |
| G5 | 前端 UI 调用 cert IPC | `src/` 下存在调用 | **FAIL** | `src/` 全目录对 `cert` **零引用**；`get_cert_status`/`install_device_cert` 无人调用 |
| G6 | `cert` 模块单元测试通过 | `cargo test` 全绿 | **PASS** | `logs/cargo-test.log`：`running 13 tests` → `test result: ok. 13 passed; 0 failed`（另 `unittests src/main.rs` 与 `Doc-tests` 均 0 tests / ok）。测试全为纯函数，不访问钥匙串。<br>⚠️ **限定：全绿 ≠ 行为正确** —— 这 13 个测试**抓不到 G9**：`managed_block_matches_firefox_expected_syntax`(`firefox.rs:342-348`) 只断言块内含 `autoload`，**从未断言 `default_personal_cert`**，等于把错误行为固化在测试里。**修 G9 时已补齐**：该测试改为断言两个 pref 的字面量（而非只比对自己的常量），并新增 `managed_block_upgrades_older_profile`（验证"只含 autoload 的旧块能被升级且不重复"），现为 **14 passed** |
| G7 | 移除临时验证钩子 | `TEMP VERIFICATION HOOK` 注释与其 CLI 分支出清 | **DONE** | `src-tauri/src/main.rs:14-39` = `// TEMP VERIFICATION HOOK — remove after the macOS bundle check.`，三个分支各自 `println`/`return`：`--cert-status`(:16-19)、`--cert-ensure`(:20-29)、`--cert-remove`(:30-39) —— **G2/G3 正是靠后两个 flag 才能执行，先删就没法测**；G10 的修复实测同样只能靠 `--cert-ensure` 驱动，故实际执行顺序为 **G9 → G10 → G7**（G7 必须最后）。<br>**已移除**：`main.rs` 中该段整体删除，`fn main()` 现直接从 `// Generate fingerprint on startup` 开始；源码 grep（三个 flag ＋ `TEMP VERIFICATION HOOK`）= **0 命中**；release 产物 `strings` 中三个 flag 亦均为 **0 命中**；`cargo test` **14 passed**，无新增告警 |
| G8 | 平台能力矩阵符合预期 | macOS=`full`、Windows=`full`、Linux=`unavailable` | PASS(代码事实) | `cert/mod.rs:418-431`；`cert/linux.rs:52-56` 直接返回 `Unsupported` |
| G9 | Firefox `user.js` 托管块是否包含 `security.default_personal_cert`（**D5 已实测其必需**） | 托管块含该 pref | **FAIL（实缺陷）→ 已修复 PASS** | `cert/firefox.rs:29` 只定义了 `OSCLIENTCERTS_PREF = "security.osclientcerts.autoload"`，`managed_block()`（:134-139）也只写这一个 pref，**全文件无 `default_personal_cert`**。模块头注释 :4-12 声称"写该 pref 即可让 Firefox 不再失败"，与 D2/D5a/D5b 四次实测**相反**；D5c 证明必须补 `default_personal_cert = "Ask Every Time"` 才会弹框并提交 `devops-client`。<br>**修复（`cert/firefox.rs`）**：新增 `DEFAULT_PERSONAL_CERT_PREF` / `ASK_EVERY_TIME` 两个常量，`managed_block()` 一并写入两个 pref；模块头两处与实测相反的论断已改写（原称"写该 pref 即可让 Firefox 不再失败"）。**14 tests passed**，新增的 `managed_block_upgrades_older_profile` 覆盖"旧块只含 autoload 时能被升级" |
| G10 | `--cert-ensure` 安装后是否**同时建立证书信任**（浏览器肯提供该身份的前提） | 安装动作包含信任设置 | **FAIL（部署阻塞级）→ 已修复 PASS** | `src` 全文搜 `TrustSettings`/`add-trusted-cert`/`SecTrustSettings`/`trustRoot` → **0 处命中**；`macos.rs:143-149` 的 `install_certificate()` 只做 `add_to_keychain(None)`，且 `:74` 走 `build_self_signed`（自签，C1 实测 `issuer_CN=devops-client`）。G3 **实证**：重建出的新身份即 `(CSSMERR_TP_NOT_TRUSTED)`。串联 B 段自己的结论「未受信任的身份不会被提供给服务器」→ **全新机器上装完证书，浏览器仍然永远拿不到，设备绑定必然失败**，除非人工 `add-trusted-cert`（B3 的修复本身就是人工做的）。<br>注：若生产改由 MDM 预置根 CA、证书由 CA 签发则不适用；但当前实现是**每设备自签**，前置条件不成立。`macos.rs:10-15` 注释只提了钥匙串授权弹框需要 MDM，**完全没提信任设置**，需一并补正。<br>**修复（`cert/macos.rs`）**：模块头补 "Trust settings" 段落（自签证书必须显式建立信任 + 浏览器按信任过滤身份 + 本机实测依据）；`install_certificate()` 改为返回 `SecCertificate`，把刚入库的对象交给下一步；新增 `trust_certificate()` = `TrustSettings::new(Domain::User).set_trust_settings_always(cert)`；`ensure()` 在安装后调用它，**失败降级为 `Partial` 而不硬失败**（理由：`renew()` 先删后建，硬失败会让设备一证不剩；且 `ensure_cert_bound()` 在登录路径上，硬失败会挡住登录）。<br>**实测（决定性）**：与 G3 **同机、同一条 `--cert-remove` → `--cert-ensure` 路径**，唯一变量就是新增的信任调用 —— G3（旧代码）`devops-client (CSSMERR_TP_NOT_TRUSTED)` 且不在 `Valid identities only`；修复后 **`devops-client` 无错误、进入 `Valid identities only`**，`capability: Full`（未降级、无 stderr 告警），user 域出现 `devops-client` 信任记录，admin 域仍为空。<br>**残留差异（已记录未收紧）**：新记录是"全域信任"（`Number of trust settings : 0`，NULL 语义），B3 人工那条是"仅 SSL"（`Policy OID: SSL`）；crate 只提供 `set_trust_settings_always`，收紧需手搓 `CFArray`。风险低 —— `build_self_signed` 用 `CertificateParams::default()`，rcgen 默认 `IsCa::NoCa` 且未设 EKU，是非 CA 叶子证书，全域信任实际只信任这一张 |

> **G1 附带的重要契约发现（直接关系 H1）**：`cert/mod.rs:74` 注释写明
> `fingerprint` 是 **SHA-256 of the DER certificate as lowercase hex — the value the server stores**，
> G1 实测输出（`05d04bb4…0a66`）确为 SHA-256。
> 而本清单 A1/C2 及 `security find-identity` 用的是 **SHA-1**（`766DA371…`）。
> → **H1「服务端按指纹落库 / 比对」必须先确认服务端口径就是 SHA-256**；若拿 SHA-1 去比，设备绑定必然不匹配。

> **G10 的判定强度分层（如实标注）**：**缺陷本身 = 实测确证**（安装路径不含任何信任调用；新装证书落
> `CSSMERR_TP_NOT_TRUSTED`）；**浏览器侧后果 = 依 B 段既有结论推定**（未受信任的身份不会被提供给服务器），
> 本轮没有在"证书未受信任"的状态下再启一次探针去直接观测浏览器，所以这一层没钉死。
> 修复侧的验证**则是实测的**：同一台机器、同一条路径、同一条命令，身份从"不在 `Valid identities only`"
> 变为"在"，唯一变量就是新增的信任调用。两层强度不同，不要混为一谈。
>
> **信任域语义（本轮踩过一次，务必记住）**：`security dump-trust-settings` **不带参数就是 user 域**，
> `-d` 才是 **admin** 域，`-s` 是 system 域（依据是该命令自身的 usage 文本）。本轮一度读反并已更正；
> 好在结论不受影响 —— B3 的 `add-trusted-cert -r trustRoot -p ssl`（无 `-d`）写的就是 **user 域**，
> 所以"默认 dump 里能看到 `devops-client`"与"删证书后该条目消失"两处观测始终自洽。台账留有更正记录。

---

## 8. H 段 — 服务端（不在本仓库内，无法自行验证）

| ID | 验证内容 | 判定标准 | 状态 | 证据 |
|----|----------|----------|------|------|
| H1 | `POST /api/auth/bind-cert` 接受指纹/序列号并落库 | 服务端返回成功且库中有绑定记录 | BLOCKED(仓库外) | 客户端调用点 `src-tauri/src/auth.rs:272-310` |
| H2 | `POST /api/auth/renew-cert` 续期链路 | 同上 | BLOCKED(仓库外) | `auth.rs:279-284` |
| H3 | `POST /api/auth/device-status` 上报 `certCapability` | 服务端据此分级 | BLOCKED(仓库外) | `auth.rs:334-340` |
| H4 | `app.device-binding` / `cert-mode=enforce` 下无证书被拒 | 无证书请求返回拒绝 | BLOCKED(仓库外) | 契约注释 `cert/mod.rs:61`、`cert/mod.rs:486-493` |

---

## 9. Z 段 — 收尾（验完必做，否则下次又是烂摊子）

| ID | 内容 | 状态 |
|----|------|------|
| Z1 | 关闭遗留 Firefox 测试实例（`--profile /tmp/fw-tls/profile`，round6 后为后台常驻），**按 PID 精确 kill** | **DONE**（D5c 用实例 `pid=4279`，先核对命令串匹配再 `kill 4279`，已确认退出） |
| Z2 | 停掉遗留探针（`probe.py` / 8443 `s_server`，按 PID） | **DONE**（`pid=4277` @18753，先核对命令串匹配再 `kill 4277`，18753 端口已释放；`8443 s_server` 更早已停） |
| Z3 | 决定是否回退钥匙串信任设置（`security remove-trusted-cert /tmp/fw-tls/client.pem`） | TODO |
| Z4 | 清理 `/tmp/fw-tls`（`round5`/`round6`/`pcontrol2` 为证据源，**清理前先把内容并入台账**） | TODO |
| Z5 | 处理 `src-tauri/src/main.rs` 的 TEMP 钩子（=G7） | **DONE**（随 G7 一并移除并验证，证据见 G7 行） |
| Z6 | 本仓库当前未提交改动较多（`cert/` 为新目录，`Cargo.toml/.lock`、`auth.rs`、`commands.rs`、`lib.rs`、`main.rs`、`panel.js`、`styles.css`、`locales/*` 均已修改），验证收尾时一并确认 | TODO |
| Z7 | 处置 Chrome 的 LaunchServices 污染（**不属本项目改动范围，需用户确认**）：僵尸 `pid=76553`（`type=Foreground`，Sep 22 09:47 起）与 `pid=11871`（Sep 25 07:54 起）都是挂死的 headless `--screenshot` 实例，导致 `open -a "Google Chrome"` 与点 Dock 图标命中**无窗口**实例。按 PID 精确 kill（先核对命令串） | TODO（待用户确认） |

---

## 10. 当前结论

**一句话：证书实体、钥匙串身份、系统 TLS 栈全部 PASS（已用新仪器 + 正对照闭环）；浏览器侧只有"人点一下"的问题，不是证书的问题。**

| 段 | 结论 | 性质 |
|----|------|------|
| A / C | 证书+私钥在钥匙串、指纹自洽、系统 TLS 栈能提交 `devops-client` | **PASS（闭环，永不复验）** |
| B | 修复前 `CSSMERR_TP_NOT_TRUSTED` → `add-trusted-cert` 后进入 `Valid identities only` | PASS（修复有效；回退命令见 B3） |
| D | Firefox 在**产品默认配置**（只写 `osclientcerts.autoload`）下 **FAIL 且与本项目证书无关**：只被要求出示 `CN=devops-client`，却提交 `www.dingtalkcs.com`；删掉自加 pref 后不变。**加上 `Ask Every Time` 后 D5 PASS**：弹框出现、人工选中、服务端收到 `CLIENT-CERT subject_CN=devops-client` | **FAIL（默认配置，定案）→ 但已有确定修复路径（D5 PASS）** |
| E | Safari 连得上，但握手挂起在证书选择 → 需要**人工点一次** | BLOCKED |
| F | Chrome **PASS（结案）**：默认配置下弹**一次**证书选择框、人工选中 `devops-client`，服务端随即收到该证书；**无需任何 pref**。对照 Firefox 默认配置**连框都不弹**、直接提交钉钉证书 | **PASS** |
| G | 三条 CLI 路径实测通过（**G1/G2/G3 PASS**），单测 **14 passed**；发现并**当场修掉两个实缺陷 —— G9：`cert/firefox.rs` 缺 `default_personal_cert`；G10：安装后从不建立证书信任（部署阻塞级）**，G10 的修复已用"同路径对照 G3"实测闭环。仅剩 `src/` 前端对 cert IPC **零引用**（G5 FAIL）与 G7（移除临时钩子，必须最后做） | 两个实缺陷已修复并实测；G5 待处理 |
| H | 服务端接口不在本仓库 | BLOCKED |
| R | **"反复验证"的真正根因是旧仪器**（stderr 块缓冲 + `ACCEPT` 非计数 + 无对端归因），不是记不住结论 | 已修 |

> **身份纪元说明（2026-09-28 15:00，G10 实测之后）**：本机 `devops-client` 身份共换了两次 ——
> 初代 `766DA371…` / SHA-256 `05d04bb4…0a66` / serial `3a476869…f322c9`（A~F 段使用）
> → 二代 `17737DFC…` / SHA-256 `39c24b83…6261` / serial `180ca13f…129c`（G3 产生）
> → 三代 `17C3D90552F9925ECE6128FEC763EDE5D97C3A13` / SHA-256 `74216f33…f40b7` / serial `45ba4632…321a7c`（G10 实测产生，**当前在用**）。
> A~F 段的浏览器结论**不依赖具体密钥**（讲的是"身份受信任且 CN 正确"时浏览器的行为），不受影响；
> 但文档中引用旧 SHA-1 的证据串需按此对应。机器当前状态为**受信任可用**：user 域信任 2 条
> （含 `devops-client`，全域语义）、`find-identity -v -p ssl-client` = **2 valid identities**。

**给产品侧的直接含义**（不是验证项，是结论推论）：

1. 若只在客户端写 `security.osclientcerts.autoload = true`，**Firefox 默认 `Select Automatically` 会挑走别的证书**（已实测）→ 服务端只会收到错误证书或空证书。
2. 因此 Firefox 侧必须在安装时同时写 `security.default_personal_cert = "Ask Every Time"`（弹框交给人选）。
   `d5/D5c-ASK` 已实测**端到端成立**：弹框出现 → 选中 `devops-client` → 服务端收到 `CLIENT-CERT subject_CN=devops-client`。
   代价是每次服务端索要证书都会弹框一次（可用「记住此决定」缓解）；`Select Automatically` 那一档在本机**已证实不可用**。
3. **Chromium 系（Chrome/Edge）是本机最省事的路径**：不写任何 pref、不做任何预配置，`F2`+`F3` 实测即为
   「弹框一次 → 人工选中 `devops-client` → 服务端收到该证书」，且同一浏览器内后续连接不再弹框（1 ms 送达）。
   对照 Firefox 默认配置**连框都不弹**、直接提交钉钉证书 → Chromium 不落入该失败模式（推测因其会按服务端 CA 列表收敛身份）。
4. Safari 侧仍无法自动化闭环：mTLS 的证书选择是 GUI 弹窗，`E1` 必须人工参与一次。
5. **客户端代码需补一个 pref（= G9，实缺陷）**：`cert/firefox.rs` 的托管块目前**只写** `security.osclientcerts.autoload`，
   必须同时写 `security.default_personal_cert = "Ask Every Time"`；否则按 D2/D5a/D5b 的实测，Firefox 上设备绑定是**确定失败**的。
   顺带该文件模块头注释 :4-12 的论断需修正（它声称只写 autoload 就能解决问题，与实测相反）。
6. **安装后必须建立证书信任（= G10，实缺陷，比 G9 更靠前）**：G3 已实证 `--cert-ensure` 装出来的**自签**证书落在
   `CSSMERR_TP_NOT_TRUSTED`，而 B 段已确认「未受信任的身份不会被提供给服务器」。因此在一台**全新机器**上，
   证书装好了浏览器（Firefox / Chrome / Safari 一视同仁）**也不会提供它**，设备绑定必然失败 ——
   除非有人人工执行 `security add-trusted-cert`。产品代码目前完全没有这一步（`add_to_keychain` 是唯一的安装动作）。
   若走 MDM 预置根 CA 的路线，则证书应由该 CA 签发，而非每台设备自签。

---

## 附：旧 D2 复测命令 —— **已废止，勿再执行**

> 本节基于已废止的 `openssl s_server`（绑 `*`、stderr 块缓冲、`ACCEPT` 非计数），执行它只会再造一次假空日志。
> 现行标准见 §1 的 R5–R9：`/tmp/fw-tls/round5.sh` / `round6.sh` 那种**独立短生命周期 Python 探针 + 判定前 kill 强制 flush + lsof 对端归因**。
> 保留下文仅用于理解当时的做法与踩坑。

```bash
# 0) 先确认 pref 已生效（只读，不重写）
grep -E 'default_personal_cert|osclientcerts' /tmp/fw-tls/profile/user.js

# 1) 记录装置指纹到 append-only 台账
LEDGER=/Users/devin/clientProjects/devops-client/logs/cert-evidence.log
mkdir -p "$(dirname "$LEDGER")"
RIG=$(pgrep -f 's_server -accept 8443' | head -1)
ACCEPT_BEFORE=$(grep -c ACCEPT /tmp/fw-tls/s_server.out)
echo "==== D2 $(date '+%F %T') rig=$RIG accept_before=$ACCEPT_BEFORE ====" >> "$LEDGER"

# 2) 按 PID 精确结束旧实例，再用**同一个已有 profile**启动（勿用 pkill -f）
kill 74496 2>/dev/null; sleep 2
nohup "/Applications/Firefox.app/Contents/MacOS/firefox" \
  --profile /tmp/fw-tls/profile --no-remote "https://127.0.0.1:8443/" \
  >> /tmp/fw-tls/firefox.log 2>&1 &
sleep 15

# 3) 回填证据
{ echo "\$ accept_after=$(grep -c ACCEPT /tmp/fw-tls/s_server.out)";
  echo "--- s_server.err ---"; cat /tmp/fw-tls/s_server.err; } >> "$LEDGER"
cat "$LEDGER" | tail -30
```

判定：出现 `depth=0 CN=devops-client` → D2 **PASS**（浏览器侧 mTLS 闭环成立）；出现新的 `peer did not return a certificate` → D2 **FAIL**，且因 B 段已修，FAIL 将指向 Firefox 侧 pref/授权问题（转 D3/D4），**不再回头怀疑证书本体**。
