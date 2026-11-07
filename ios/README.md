# 农场助手 iOS

按 Android 版 `0.1.3` 的功能移植的原生 iOS 应用：通过 Network Extension 抓取 QQ / 微信小游戏的农场登录 Code 与好友 GID，确认后推送到用户自己的 `qq-farm-bot` 服务器；同时支持多服务器账号管理、微信扫码登录、掉线提醒、证书管理和日志。

> 本目录是 Xcode 工程源码，未在 Windows 上编译验证。构建需要 macOS + Xcode 15 或更新版本，并且必须使用带 Network Extension 权限的付费 Apple Developer 账号在真机运行。

## 与 Android 版的功能对应

| Android | iOS | 说明 |
| --- | --- | --- |
| VPN 仅接管 QQ / 微信 | NEPacketTunnelProvider 接管全设备流量，但只解密 `gate-obt.nqf.qq.com` | iOS 没有“允许应用”列表；其他连接原样透传 |
| hev-socks5-tunnel + 本地 SOCKS5 | 相同 hev-socks5-tunnel 2.19.1 + 扩展内 SOCKS5 | 通过 socketpair 接入 NEPacketTunnelFlow |
| 本地 CA + 叶子证书镜像上游 Subject/SAN/有效期 | 相同 | 先校验真实上游证书，再签发兼容叶子证书 |
| 获取 Code / 身份 / 好友 GID | 相同 | WebSocket + protobuf 解析器逐行移植 |
| 多服务器、账号列表、在线状态 | 相同 | 并行读取，单台失败不影响其他服务器 |
| 推送 Code / 好友、新增、覆盖 | 相同 | 所有写入仍需用户确认 |
| 微信扫码登录 | 相同（展示/保存二维码、打开微信、轮询、完成登录） | 微信可能拒绝从相册识别登录码 |
| 掉线提醒（前台服务） | VPN 运行时由扩展长轮询；未运行时由 BGAppRefresh 机会式检查 | 后台能力弱于 Android 前台服务 |
| 悬浮窗（跨应用） | 应用内“快捷面板” | iOS 不允许第三方应用在其他应用上层显示窗口 |
| 阻止截屏 | 不支持 | iOS 不提供应用级截屏拦截 |
| 应用内检查并安装 APK | 检查 GitHub `ios-v*` Release 并打开下载页 | iOS 不能由应用自行安装 |
| Token 存 Android Keystore | Keychain（App 与扩展共享访问组） | 不参与 iCloud 备份 |
| Code 仅内存 5 分钟 | 相同（App Group 共享状态，扩展负责过期） | 扩展重启时开始新一轮 |

## 构建

### 环境

- macOS 13+，Xcode 15+，命令行工具已安装
- `xcodegen`：`brew install xcodegen`
- 付费 Apple Developer 账号（Network Extension entitlement）
- 真机（模拟器不能运行 Packet Tunnel）

### 一键初始化

```bash
cd ios
bash scripts/bootstrap.sh
```

脚本会：

1. 克隆并构建固定版本 `hev-socks5-tunnel 2.19.1` 的 `HevSocks5Tunnel.xcframework` 到 `Vendor/`；
2. 运行 `xcodegen generate` 生成 `FarmHelper.xcodeproj`。

然后打开工程：

```bash
open FarmHelper.xcodeproj
```

在 Xcode 中为 **FarmHelper** 和 **FarmHelperTunnel** 两个 target 选择同一个 Team。能力已在 `Config/*.entitlements` 中声明：

- App Groups：`group.io.github.ccpopy.farmhelper`
- Network Extensions：`packet-tunnel-provider`
- Keychain Sharing：`$(AppIdentifierPrefix)io.github.ccpopy.farmhelper`

首次运行需要在开发者后台为两个 App ID 开启同样能力，或让 Xcode 自动管理签名。

### 手动构建

```bash
cd ios
bash scripts/build-hev.sh
xcodegen generate
xcodebuild -project FarmHelper.xcodeproj -scheme FarmHelper \
  -configuration Debug -destination 'generic/platform=iOS' build
```

Release 归档需要真实签名；`xcodebuild -exportArchive` 需要你自备 `ExportOptions.plist`。

## GitHub Actions 编译与发布

工作流文件：`.github/workflows/ios.yml`。

- **推送 / PR**（改动 `ios/**` 或该工作流）：在 `macos-15` 上构建未签名 Release archive，产出未签名 IPA、dSYM 和 SHA-256 校验值，作为 Actions artifact 保留 30 天。
- **推送 `ios-vX.Y.Z` 标签**：构建后创建或更新 GitHub Release；配置了签名密钥时同时产出已签名 IPA，否则只发布未签名 IPA。
- **手动运行**：Actions → “Build and release iOS” → Run workflow。可填写版本号；勾选 `release` 会创建或更新 GitHub Release，不勾选只构建 artifact。
- 版本号优先级：标签 `ios-vX.Y.Z` → 手动输入的 `X.Y.Z` → `ios/FarmHelper/Info.plist`。构建号使用 GitHub `run_number`。
- hev-socks5-tunnel 的源码和 xcframework 会按固定版本 `2.19.1` 缓存；首次运行需要编译原生库，耗时较长。
- GitHub Release 使用 `--latest=false`，不会影响 Android 或桌面版本的 latest 通道。

### 发布标签

```bash
git tag ios-v0.1.3
git push origin ios-v0.1.3
```

发布说明从 `ios/CHANGELOG.md` 中同名的 `## ios-vX.Y.Z` 小节读取；没有小节时使用简短默认说明。Release 资产：

- `FarmHelper-ios-vX.Y.Z-unsigned.ipa`：未签名，需要用自己的证书和描述文件重签后安装。
- `FarmHelper-ios-vX.Y.Z-signed.ipa`：仅在配置签名密钥时生成；能否安装取决于描述文件类型与已登记设备。
- `FarmHelper-ios-vX.Y.Z-dSYMs.zip`：调试符号。
- `SHA256SUMS.txt`：SHA-256 校验值。

### 可选签名密钥

不配置以下 secrets 时，CI 只发布未签名 IPA。配置了证书或任一描述文件 secret 后，必须把三个必需的签名 secret 都配置齐，否则工作流会明确报错；`IOS_SIGNING_IDENTITY` 仅用于指定身份，不算必需。

| Secret | 必需 | 说明 |
| --- | --- | --- |
| `IOS_CERTIFICATE_BASE64` | 签名时 | Apple Development / Apple Distribution `.p12` 的 base64 |
| `IOS_CERTIFICATE_PASSWORD` | 否 | `.p12` 密码；没有密码时留空 |
| `IOS_SIGNING_IDENTITY` | 否 | 完整 codesign 身份名；留空时自动取导入证书的第一个身份 |
| `IOS_APP_PROFILE_BASE64` | 签名时 | 主 App `.mobileprovision` 的 base64 |
| `IOS_TUNNEL_PROFILE_BASE64` | 签名时 | `FarmHelperTunnel.appex` `.mobileprovision` 的 base64 |

两个描述文件必须分别匹配：

- App ID `io.github.ccpopy.farmhelper` 与 `io.github.ccpopy.farmhelper.tunnel`
- App Group `group.io.github.ccpopy.farmhelper`
- Keychain Sharing 组 `io.github.ccpopy.farmhelper`（两个 profile 都要包含这个共享组）
- Network Extensions `packet-tunnel-provider`（扩展 profile 必须包含；App profile 建议也包含）

签名脚本直接使用描述文件里的 `Entitlements` 进行 `codesign`，所以 capability 缺失会在构建时明确失败，而不是生成一个装不上的包。开发 / Ad Hoc 描述文件只能安装到已登记设备；面向公众分发需要 App Store 描述文件并通过 TestFlight 或 App Store 发布。

生成 base64（macOS）：

```bash
base64 -i certificate.p12 | pbcopy
base64 -i app.mobileprovision | pbcopy
base64 -i tunnel.mobileprovision | pbcopy
```

已签名 IPA 也可以在本地用 Transporter 或 `xcrun altool` 上传 TestFlight；工作流本身不自动上传。

## 使用

1. 打开应用 → 设置 → 添加服务器：填写 `https://你的服务器` 与管理 Token，保存并测试。
2. 设置 → 抓包证书 → 生成本地证书 → 生成安装描述文件 → 分享到“文件”。
3. 在“文件”中打开 `.mobileconfig`，按系统提示到“设置”安装描述文件。
4. 打开 **设置 › 通用 › 关于本机 › 证书信任设置**，为 `Farm Helper Local CA` 打开完全信任。
5. 回到应用，“抓包证书”应显示“已安装并信任”。
6. 获取页选择 QQ 或微信，点中央按钮启动 VPN；系统会弹出 VPN 配置确认。
7. 进入 QQ / 微信中的农场，登录后回到应用：结果卡片会出现 Code、身份和好友数量。
8. 点“推送到服务器”，确认目标账号后写入。
9. 账号页可查看在线状态、选择掉线提醒账号；点账号可重新登录。

### 微信扫码登录

获取页切到微信 → “扫码登录” → 打开扫码页 → 获取二维码。二维码可保存到相册，也可点“打开微信”后用另一台设备的微信摄像头扫描。扫码并在微信中确认后，应用会提示“已授权”，点“完成登录”提交到服务器。

### 掉线提醒

- VPN 运行期间：隧道扩展使用服务器的 `/api/mobile/offline-events` 长轮询；旧服务器返回 404 时回退为按“检查间隔”轮询 `/api/accounts` + `/api/status`，连续两次离线才提醒。
- VPN 未运行：使用 iOS 后台刷新（BGAppRefresh）做机会式检查，系统可能延迟或跳过。需要持续提醒时请保持 VPN 运行。
- 通知点击后会校验服务器、账号 UUID；账号已删除或替换时提示失效。

## 架构

```
FarmHelper (SwiftUI app)
  ├── 设置 / 账号 / 日志 / 获取 UI
  ├── APIClient、ServerDirectory、后台检查
  ├── CertificateAuthority（生成 CA、导出 .mobileconfig、信任自检）
  └── TunnelManager（NETunnelProviderManager）
        │  providerConfiguration: platform / friendCapture / passthrough / socksPassword
        ▼
FarmHelperTunnel (NEPacketTunnelProvider)
  ├── TunnelEngine：socketpair ↔ NEPacketTunnelFlow ↔ hev-socks5-tunnel
  ├── LocalSocks5Server：127.0.0.1，用户名 farm + 随机密码
  ├── TlsMitmHandler：校验上游证书 → 镜像叶子证书 → 解析 WebSocket
  ├── WsInspector：protobuf / RFC6455 解析 Code、自身 GID、好友 GID
  ├── CaptureCoordinator + CaptureStore（App Group 共享）
  └── ExtensionMonitor：VPN 运行期间的掉线长轮询
```

共享代码位于 `Shared/`，同时编入 App 与扩展：

- `CaptureStore`：App Group UserDefaults 中的本轮结果、generation、消费状态
- `SharedSettings`：多服务器配置、Token（Keychain）、推送与提醒开关
- `SharedLog`：脱敏、限长的双进程日志
- `TunnelStatusStore`：扩展写入的 TLS 主机与转发统计
- `CertificateAuthority`：CA/叶子证书、Keychain 共享、系统信任自检
- `APIClient` / `ServerDirectory` / `MonitorEngine`：与 Android 相同的服务器协议

## 与 Android 的实现差异

1. **流量范围**：iOS 公开 API 没有按应用接管。所有流量进入隧道，代理只对 `gate-obt.nqf.qq.com:443` 做 TLS 解密；其他主机按原始字节转发。
2. **悬浮窗**：系统不允许。提供应用内快捷面板；VPN 在后台仍由扩展保持。
3. **截屏**：iOS 不提供 `FLAG_SECURE` 等价能力。
4. **后台常驻**：扩展只在 VPN 开启时保持；VPN 关闭后应用会被挂起。提醒能力依赖 BGAppRefresh。
5. **证书安装**：必须由用户在系统中安装描述文件并手动打开完全信任；应用不能静默安装。
6. **更新**：只能打开下载页，不能像 Android 一样下载 APK 并调起安装器。
7. **文本分享导入**：Android 可从其他应用分享纯文本到助手；iOS 未做 Share Extension，改用“手动导入 → 从剪贴板粘贴”。
8. **超时配置**：Android 使用打过补丁的 hev 解析器把 `0` 转成无限等待；iOS 直接使用上游 2.19.1，因此配置为 `86400` 秒（一天），效果等同“不因空闲主动断开”。
9. **DNS**：隧道内 DNS 设置为 `223.5.5.5` 与 `119.29.29.29`，经 SOCKS UDP 转发。Android 会跟随底层网络 DNS。
10. **Token 存储**：Keychain 共享访问组，而不是 Android Keystore。

## 安全边界

- 只读取农场登录 URL 中的 `code`、登录回包中的自身 GID、`FriendService` 回包中的 `GameFriend.gid`；不读取聊天、通讯录、相册或文件。
- 上游证书通过系统信任链与主机名校验；校验失败即断开，不提供“忽略证书错误”开关。
- 叶子证书只复制上游证书的 Subject、DNS SAN 与有效期，不复制其公钥或签发者。
- 只获取 Code 的模式会返回 `451` 阻止官方登录先消费 Code；微信“只读好友”模式转发登录但不保留 Code。
- Code 有效期为 5 分钟，推送成功后按 generation 清除；跨进程结果快照使用 Keychain 中的设备专属密钥加密后写入 App Group；日志脱敏并限制 200 条。
- 不提供证书固定绕过、不修改 QQ / 微信、不要求越狱。

## 已知限制

- 微信 VPN 获取仍是实验性功能：微信可能使用独立网络栈或证书固定，握手失败时日志会显示 `客户端 TLS 握手失败`。
- iOS 的后台提醒依赖系统调度；VPN 未运行时可能长时间不检查。
- 服务器必须支持 Android 版使用的同一套 API（`/api/accounts`、`/api/status`、`/api/friend-known-gids/batch-add`、`/api/wx-login/*`、`/api/mobile/offline-events`）。
- `gate-obt.nqf.qq.com` 的证书或协议变化会导致解析失效；届时需要更新 `WsInspector` 与证书镜像逻辑。
- 未在真机验证：本工程在 Windows 上编写，无法运行 Xcode 编译或真机测试。

## 目录

```
ios/
├── project.yml                 # XcodeGen 工程定义
├── FarmHelper/                 # SwiftUI 应用
├── FarmHelperTunnel/           # Network Extension
├── Shared/                     # 双 target 共享代码
├── Config/                     # entitlements / 隐私清单
├── scripts/                    # hev、工程生成、archive、签名、打包
├── Vendor/                     # 构建产出的 HevSocks5Tunnel.xcframework（不入库）
├── CHANGELOG.md                # iOS 发布说明
└── README.md
```

## 版本

- 应用版本：`0.1.3`，与 Android 当前正式版对应；本地构建号 `1003`，GitHub Actions 使用 `run_number` 作为构建号。
- 部署目标：iOS 16.0。
- 第三方：`hev-socks5-tunnel 2.19.1`（GPL-3.0，见其仓库 LICENSE）。
