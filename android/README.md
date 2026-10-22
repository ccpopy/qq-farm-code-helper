# 农场助手 Android（开发版）

原生 Java / Material Components Android 应用，不依赖 Android Studio。
当前保持 `versionName 0.1.0`、`versionCode 1`，尚未正式发布。

## 构建与覆盖安装

需要 JDK 17、Android SDK 命令行工具、`platforms;android-36` 和
`build-tools;36.0.0`。首次下载依赖需要联网；不要把本机 SDK 或签名密钥提交到仓库。

在仓库根目录执行：

```powershell
.\scripts\build-android.ps1 -JdkDirectory $env:JAVA_HOME -SdkDirectory $env:ANDROID_HOME
.\scripts\build-android.ps1 -Install -DeviceSerial <已授权设备序列号> -AdbPath <adb路径>
```

脚本验证固定的 hev-socks5-tunnel 2.19.1 AAR 的 SHA-256，随后执行 lint
和 APK 构建。默认下载直连失败才使用 HTTPS 加速源。APK 输出到忽略目录
`outputs/android/`。`adb install -r` 是覆盖安装，保留配置和本地证书；不要卸载或清数据。

## 使用与安全边界

1. 填写自己的 Node 部署服务器 URL 和管理 API Token，测试连接并选择目标账号。
   新账号使用“新增服务器账号并同步”，不要求它预先存在。
2. 先开启“全量透传诊断”确认 QQ 农场能正常进入。此模式不解密，也不会获取 Code。
3. 获取模式需要用户手动生成、导出并在系统安装本机 CA。安装 CA 不代表 QQ 一定信任；
   不修改 QQ、不 root、不绕过证书固定或 TLS 校验。
4. 用户授权 VPN 后，仅将 QQ 应用的新 TCP/UDP 流量送入本机转发，包括 DNS；
   不按固定网关 IP 筛选入口，不要求重启 QQ。获取模式只解密精确的农场目标 SNI，
   其他连接原样转发。VPN 两分钟自动结束，也可随时手动停止。
5. Wi-Fi 可能只有 IPv4，蜂窝网络也不保证支持 IPv6。以当前默认互联网连接为准，
   检测 IPv6 地址及默认路由，跟随网络变化重新配置路由、DNS、MTU 和受保护套接字。
   不强制开关 Wi-Fi、移动数据，也不主动选用备用蜂窝网络。
6. 获取后核对账号再推送；已有账号按服务器返回的 ID 更新，不按备注猜测。
   有自身 GID 时阻止更新身份不匹配的服务器账号。好友同步为增量追加。
   新账号备注会附显式随机后缀，避免上游同名创建变成覆盖已有账号。
7. 服务器接受更新和账号真正在线是两件事，可分别查看结果并检查登录状态。

Token 由 Android Keystore 加密，不回显、不参与系统备份。Code 仅在内存保留五分钟，
成功提交后按请求对应的数据代次清除。日志最多 200 条，仅驻留内存并脱敏。
TLS 主机元数据仅限本次内存，不包含路径、参数或正文，不随日志导出；不保存原始包。
界面禁止普通截屏与录屏。

应用可移除本地 CA/私钥；系统已安装的 CA 必须由用户在 Android 的凭据设置中移除。
卸载应用或删除本地 CA 不会自动删除系统证书。请对照证书名称和指纹操作。

## 权限

- 网络和网络状态：连接用户服务器、转发 QQ 流量、检查更新、识别底层网络。
- VPN：由系统单独确认；会替换当前运行的其他 VPN。
- 前台服务：VPN/用户启用的账号检查各有持续通知和停止操作。
- 通知：显示获取结果、账号连续离线或服务器 Token 失效；可拒绝或撤销。
- 安装未知应用：仅在用户下载并选择安装更新时交给系统授权，不静默安装。
- 不申请通讯录、聊天读取、无障碍服务、root 或广泛存储权限。

## 服务器离线提醒

适用于 Node 直接部署，不依赖 Docker。当前使用用户明确启用的服务器状态轮询，
默认 60 秒、连续两次有效离线结果才提醒。断网/请求失败会重置计数；401/403
表示管理 Token 失效，不伪报 QQ 账号掉线。休眠与省电限制可能延迟提醒。

上游有 webhook，但手机通常没有公网入站地址，而且通知载荷缺少可靠账号 ID。
第三方通知中转、锁屏实际投递和后台长时间稳定性暂不视为验收通过。

## 更新与正式发布规范

Android 更新只识别 `android-vX.Y.Z` 的非草稿、非预发布 GitHub Release，资产名必须是
`QQFarmCodeHelper-android-vX.Y.Z.apk`。下载验证资产 SHA-256、包名、版本、递增
versionCode 和 APK 签名，随后由用户确认安装。更新源不接收服务器 Token 或 Code。
加速源仍须 HTTPS；它影响可用性与隐私，不能替代 APK 签名验证。

正式发布必须使用持续保管的独立签名密钥，通过环境变量 `ANDROID_KEYSTORE_PATH`、
`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD` 配置，
不要将密钥或口令写进源码。调试版无法直接覆盖为不同签名的正式版。
创建 Android Release 时设置 `make_latest: false`，避免影响桌面应用的 latest 更新通道。
未获得发布授权不创建 Release，也不为测试增加版本。

## 当前已知问题与继续排查

这次是开发中源码提交，不代表全部功能已验收。蜂窝网络下已观察到真实 QQ 农场的
加密连接和上游响应，且无需重启 QQ。当前 IPv4-only Wi-Fi 仍可能出现“平台登录失败”，
原因尚未确认；不能仅凭 VPN 就绪、DNS 可用或助手自身 HTTPS 成功判断 QQ 已可用。
强行启用底层不支持的 IPv6 不能解决此问题。

真实 QQ 的 CA 信任、Code/好友 GID 获取以及更新服务器后确认重新在线，尚未走通完整链路。
通知投递、IPv6-only/NAT64、其他 Android 版本及厂商兼容性也需后续确认。

该分支只保留应用源码、构建文件及必要说明；不包含本机测试脚本、虚构数据、
测试 APK、截图、抓包、设备日志、服务器凭据或本机 SDK 路径。
