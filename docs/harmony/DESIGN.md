# 鸿蒙版 DD-up：设计与协议（D27）

```
鸿蒙 App (ArkTS) ──加密 WebSocket（局域网 / Tailscale）──▶ ddup-bridge (Mac) ──stdin/stdout──▶ hc connect ──▶ herdr-core
                                                              └─ 推送监视：hc connect（plain）events.subscribe ─▶ Push Kit V3
```

## 1. 组成

| 部分 | 位置 | 作用 |
|---|---|---|
| `ddup-bridge serve` | `bridge/` | 常驻服务：WebSocket 监听、握手、设备认证、转发 `hc connect`、推送监视 |
| `ddup-bridge pair` | 同上 | 生成一次性配对码，在终端打印二维码和配对链接 |
| `ddup-bridge devices` / `revoke` | 同上 | 列出和吊销已配对设备 |
| `ddup-bridge push-test` | 同上 | 给已登记推送的设备发一条测试通知 |
| 鸿蒙 App | `mobile/harmony/` | 扫码配对、Agent 列表、终端、快捷键、发送、推送登记 |

状态目录默认是 `~/Library/Application Support/DD-up/bridge/`，可以用 `--state-dir` 或 `DDUP_BRIDGE_HOME` 改。测试一律用项目内的目录和测试 core（`hup-dev`）。

| 文件 | 内容 | 权限 |
|---|---|---|
| `identity.json` | 桥接服务的 X25519 静态私钥和公钥 | 0600 |
| `devices.json` | 设备 id、名称、令牌 SHA-256、推送 token、时间戳 | 0600 |
| `pairing.json` | 待用的配对码（只存 SHA-256）、过期时间、失败次数 | 0600 |
| `config.json` | 监听地址、`hc` 路径、session、推送配置 | 0600 |
| `push-key.json` | 华为服务账号密钥文件（用户放入） | 0600 |

## 2. 配对链接（二维码内容）

```
ddup://pair?v=1&h=<host>&h=<host2>&p=<port>&k=<bridge 静态公钥>&c=<配对码>&n=<名称>
```

- `k`：32 字节 X25519 公钥，base64url 无填充。App 用它固定桥接服务的身份，防中间人。
- `c`：16 字节随机数，base64url 无填充。一次性、10 分钟内有效；同一个码失败 5 次就作废。
- `h` 可以出现多次（局域网地址、Tailscale 地址），App 按顺序尝试。
- `n`：显示名称，URL 编码。

## 3. 连接与握手（DDB1）

WebSocket 地址 `ws://<host>:<port>/v1`。所有消息都是二进制帧。

1. App → 桥接服务：`"DDB1"`（4 字节 ASCII）‖ `ce`（App 临时公钥，32 字节）。
2. 桥接服务 → App：`se`（桥接服务临时公钥，32 字节）。
3. 双方计算：
   - `dh1 = X25519(ce_priv, bs_pub) = X25519(bs_priv, ce_pub)`（证明桥接服务持有静态私钥）
   - `dh2 = X25519(ce_priv, se_pub) = X25519(se_priv, ce_pub)`（前向保密）
   - `th = SHA-256("DDB1" ‖ bs_pub ‖ ce_pub ‖ se_pub)`
   - `okm = HKDF-SHA256(ikm = dh1 ‖ dh2, salt = th, info = "ddup-bridge v1 keys", L = 64)`
   - `k_c2s = okm[0..32]`，`k_s2c = okm[32..64]`
4. 之后每条记录的格式是 `u32 大端长度 ‖ u64 大端计数器 c ‖ AES-256-GCM(key, nonce = 0x00000000 ‖ c, aad = 空)`。长度之后依次是计数器、密文和 16 字节 tag。
   - 每个方向的计数器从 0 开始，每发一条加 1。
   - 接收方只接受比上一条更大的计数器，所以重放和乱序都会被拒绝；中间允许有空缺，因为鸿蒙的 `ws.send` 偶尔会发送失败（真机日志里看到过 `ws.send failed`）。
   - 解密失败就断开连接。
   - 接收方把 WebSocket 消息当作字节流，按长度前缀拆出记录。因为鸿蒙的 WebSocket 可能把一条消息分几次回调（存在 `dataEnd` 事件），也可能连着到达。单条记录上限 4 MiB。
   - 测试向量（固定密钥、首条记录的 sealed 部分）见 `bridge/testdata/vectors.json`，生成方式是 `cargo test vectors -- --nocapture`。

5 秒内没完成握手和认证，桥接服务会断开。

## 4. 明文消息

第一个字节是类型：

| 类型 | 内容 |
|---|---|
| `0x00` | `hc connect` 的原始字节（两个方向）。认证成功前发会被断开。 |
| `0x01` | UTF-8 JSON，桥接服务控制消息 |

### 控制消息

| 方向 | 消息 | 说明 |
|---|---|---|
| App→ | `{"t":"pair","code":"…","device":"Mate 70"}` | 用配对码配对 |
| ←桥 | `{"t":"paired","device_id":"…","token":"…"}` | App 保存 `device_id` 和 `token`（存进 Asset Store）。之后等同认证成功 |
| App→ | `{"t":"auth","device_id":"…","token":"…"}` | 已配对设备每次连接时发 |
| ←桥 | `{"t":"ok","bridge":"0.1.0","name":"…"}` | 认证成功，桥接服务启动 `hc connect`，开始转发 `0x00` |
| ←桥 | `{"t":"error","code":"unauthorized"\|"pair_expired"\|"pair_invalid"\|"hc_failed","msg":"…"}` | 随后断开（`hc_failed` 除外） |
| App→ | `{"t":"push_token","token":"…"}` | 登记或更新华为 Push Token；空串表示关闭推送 |
| App→ | `{"t":"foreground","on":true}` | App 在前台时，桥接服务不给这台设备发推送 |
| App→ | `{"t":"push_test"}` | 立即发一条测试推送 |
| ←桥 | `{"t":"push_result","re":"push_test"\|"push_token","ok":true,"detail":"…"}` | `re` 指明是哪条请求的结果 |
| ←桥 | `{"t":"hc_closed","code":0}` | `hc connect` 退出了；App 应重连 |
| App→ | `{"t":"transcript","id":1,"app":"claude-code","cwd":"…","pid":123,"file":"…","offset":0}` | 读 Agent 自己的会话记录（D32）。`file`/`offset` 是上次回复里的，第一次不带 |
| ←桥 | `{"t":"transcript","id":1,"file":"…","offset":…,"reset":true,"truncated":true,"items":[{"k":"user"\|"assistant"\|"tool"\|"result","text":"…","name":"Bash","detail":"…","id":"…","err":false}]}` | `reset` 表示换了文件或第一次读，App 清空重来；`result` 用 `id` 对应它的 `tool`；出错时只有 `error` |
| App→ | `{"t":"ports","id":2}` | Mac 上在监听、能回 HTTP 的端口 |
| ←桥 | `{"t":"ports","id":2,"sites":[{"port":5173,"process":"node","pid":…,"cwd":"…","title":"…","status":200}]}` | 去掉 cwd 为 `/` 的系统服务；有标题的排前面 |
| App→ | `{"t":"forward","id":3,"port":5173}` | 为这个端口开转发 |
| ←桥 | `{"t":"forward","id":3,"port":5173,"listen_port":54321,"token":"…"}` | App 打开 `http://<连上的那个地址>:<listen_port>/路径?__ddup=<token>` |

带 `id` 的请求在后台处理，回复按 `id` 对应，不挡终端数据。

### 会话记录（`bridge/src/transcript.rs`）

- 找文件：先按 core 报的前台进程 `foreground.pid` 找——Claude Code 读 `~/.claude/sessions/<pid>.json` 里的 `sessionId`，Codex 用 `lsof -p` 找它开着的 `rollout-*.jsonl`；找不到再取这个目录最新的会话（Claude：`~/.claude/projects/<cwd 非字母数字换成 ->/`；Codex：最近 7 天、`session_meta.cwd` 相同）。终端的 shell 目录和 Agent 的目录可能不同，所以 App 用的是前台进程的目录（`foreground.cwd`），桌面版也是这样。
- 第一次读文件末尾 1.5 MB，最多 150 条；之后只读新增的整行。
- 保留：用户的话、Agent 的文字、工具调用（一行摘要 + 详情）、工具结果（截到 2000 字）。丢掉：thinking/reasoning、系统注入的内容（`<…>` 开头、AGENTS.md）、子 Agent 的记录、元数据。
- 摘要：Bash 用 `description`（命令放详情），Edit 显示文件、详情是 -/+ 行，Codex `exec` 从 JS 里取出 `cmd:"…"`，`apply_patch` 列出改的文件；Codex 的 `exec_command` 结果从 JSON 里取 `output`，非 0 退出码标成失败。
- 格式没有公开文档，不认识的记录直接跳过。

### 网页预览转发（`bridge/src/ports.rs`）

- 每个目标端口一个转发，监听随机端口，和桥接服务一样只接受局域网、Tailscale 和本机地址；桥接服务退出时一起关掉。
- 第一次请求必须带 `?__ddup=<token>`：回 302 和 `HttpOnly; SameSite=Lax` cookie，并去掉这个参数；之后每个请求都要带这个 cookie，否则返回 403。令牌比较用常数时间。
- 转发前把 `Host`、`Origin`、`Referer` 改成 `localhost:<port>`（vite 等会检查），去掉转发自己的 cookie；普通请求加 `Connection: close`，一个连接一个请求；WebSocket 升级（热更新）原样双向转发。
- 先连 `127.0.0.1`，不通再连 `[::1]`。

## 5. App 侧说的 core 协议（与桌面版相同）

见 `.work/herdr-app/src/hc.rs` 和 `scripts/dev hc schema`：

- 先发一行 `{"jsonrpc":"2.0","id":0,"method":"hello","params":{"framed":true}}\n`，读一行回复。
- 之后每帧：`u32 LE channel` ‖ `u8 type` ‖ `u32 LE len` ‖ payload。type：DATA=0, CREDIT=1, CLOSE=2, RESIZE=3, FOCUS=4, FACTS=5。
- channel 0 的 DATA 是 JSON-RPC 行。
- `events.subscribe` 回复里的 `channel` 上是 JSON 事件行（`event` / `missed`），要按读取的字节数发 CREDIT。
- `terminal.attach {terminal, cols, rows}` 回复里的 `channel`：先是首屏，之后是实时字节；按键作为 DATA 发到同一个 channel；RESIZE 的 payload 是 `u16 LE cols ‖ u16 LE rows`。
- 状态和已读：`docs/labels.md`（`.work/herdr-app/docs/labels.md`）。

## 6. 推送规则

推送监视用单独一条 `hc connect`（plain 模式），调用 `events.subscribe`，维护每个终端的 `status.state`、`turn` 和 `labels`。

- 进入 `blocked` 或 `error`（之前不是）：推送"需要你处理"。正文用 `status.msg`，没有时用 `kind`（permission / question / auth）。
- 收到 `turn` 事件、状态不是 blocked/error，并且 `herdr.seen` 缺失或小于新的 turn：推送"已完成"。
- 从没报告过 status 的终端（不是 Agent）不推送。
- 任何一台设备在前台连接时，不给这台设备推送。
- `notifyId` 由终端地址的哈希得出，同一个终端的新通知覆盖旧通知。
- 请求：`POST https://push-api.cloud.huawei.com/v3/{project_id}/messages:send`，头部 `Authorization: Bearer <JWT>`、`push-type: 0`。
  - JWT：header `{"kid":key_id,"typ":"JWT","alg":"PS256"}`，payload `{"iss":sub_account,"aud":"https://oauth-login.cloud.huawei.com/oauth2/v3/token","iat":…,"exp":iat+3600}`，用 `private_key` 做 SHA256withRSA/PSS 签名。
  - body：`payload.notification{category,title,body,clickAction{actionType:0,data{terminal}},notifyId}`，`target.token[]`，`pushOptions{testMessage,ttl}`。
  - `category` 默认 `MARKETING`；申请到自分类权益后在 `config.json` 里改。`testMessage` 默认 true（调试阶段）。

## 7. 鸿蒙端要点

- 网络：`@ohos.net.webSocket`，权限只需 `ohos.permission.INTERNET`。
- 加密：`@kit.CryptoArchitectureKit`，用到 X25519（API 11+）、HKDF（API 12+）、AES-GCM、SHA256。
- 终端：ArkWeb 加载 rawfile 里的 xterm.js，通过 `WebMessagePort` 和 ArkTS 双向传数据。
  - HarmonyOS 7 已知问题：调用 `focus()` 不弹软键盘；华为输入法只发 `insertText`。所以输入不交给 xterm 处理，改用 ArkUI 原生输入框加快捷键栏。
- 扫码：Scan Kit 默认界面（`scanBarcode.startScanForResult`）。也可以粘贴配对链接。
- 推送：`pushService.getToken()`。前提：在 AppGallery Connect 建项目、开通推送服务，`module.json5` 里配置 `client_id`，并用关联该应用的签名。
- 后台：App 退到后台后连接会被挂起。回到前台时重连并重新订阅；后台提醒全靠推送。

## 8. 验收（四件事分开记录）

| 项 | 怎么验 |
|---|---|
| 桥接服务静态检查和单元测试 | `scripts/test`（cargo test、clippy） |
| 桥接服务和 core 联调 | `tests/integration/bridge-e2e`：用测试 core 和 Rust 测试客户端走完配对、认证、hello、`terminal.list` |
| 推送 | JWT 签名本地自验；真实发送要用户提供服务账号密钥后跑 `ddup-bridge push-test` |
| 鸿蒙 App 编译 | 需要 DevEco Studio（本机未安装），由用户编译 |
| 鸿蒙 App 真机 | 人工验收清单 `tests/gui/HARMONY_CHECKLIST.md` |
