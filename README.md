# QQ农场 Code 获取助手

一个面向 Windows QQ 客户端的小型 Rust/Tauri 应用。它会在本机临时启用定向代理，仅解密 `gate-obt.nqf.qq.com:443`，从 QQ 农场的 `/prod/ws` 登录请求中提取一次性 `code`，并在请求到达腾讯服务器前阻断它。

获取后可以：

- 读取 Windows QQ 主界面当前昵称，并只读 QQNT 登录列表来唯一匹配 QQ 号和头像；
- 自动调用 `qq-farm-bot` 的 `POST /api/accounts` 新增 QQ 账号；
- 或者仅保存在内存中，由用户点击复制。

## 参考项目与关系说明

- 参考并配套使用：[liyangpengs/qq-farm-bot](https://github.com/liyangpengs/qq-farm-bot)
- 本项目围绕该仓库提供的账号管理接口和 QQ 农场登录场景开发，用于在 Windows QQ 本地获取一次性 Code 并同步到服务器。
- 本项目是独立的第三方辅助工具，不属于 `qq-farm-bot` 官方组件，不要求修改或覆盖原作者仓库代码。
- 参考项目的代码、名称及相关权利归原作者所有；使用时请同时遵守对应仓库的许可说明和相关服务条款。

## 使用流程

1. 在服务器设置中填写 `qq-farm-bot` 管理页面地址。
2. 在 `qq-farm-bot` 左侧边栏底部复制当前登录 Token，填入工具。
3. 点击「测试连接」，确认 `/api/user/me` 可以正常返回当前用户。
4. 保存设置并点击「启动获取」。
5. 代理启动后，再打开或登录 Windows QQ；若 QQ 已在运行，建议完全退出后重新打开。等待工具显示账号已确认，再进入 QQ 农场。
6. 工具捕获 Code 后会立即恢复系统代理、移除临时根证书，再同步到服务器。

QQ 农场窗口当次出现网络错误是预期现象：官方的 WebSocket 登录请求被本地工具主动阻断，避免一次性 Code 被官方客户端先消耗。

## QQ 身份识别

- 工具通过 Windows UI Automation 读取新版 QQ 主窗口左上角当前昵称，再与 `%APPDATA%\QQ\auth\login.enc` 的 `uin`、`nickName` 和 `faceUrl` 做唯一匹配，不读取聊天数据库。
- `isUserLogin` 和最近活动分区可能指向后台主会话，不能代表界面当前切换的账号，因此不再用于自动绑定。
- 界面每 2 秒自动检测一次切号，并要求连续两次结果一致；切换过程中、昵称同名、主窗口不可读或登录列表尚未刷新时会立即清空旧身份。
- 启动后等待农场登录期间，后端会持续检测 QQ；新账号连续稳定后会自动替换锁定身份，因此可以先启动获取、再切换目标 QQ、最后进入农场。
- 捕获 Code 后还会再次确认；若捕获前后的账号仍不一致，则停止自动同步并保留 Code。
- 启用自动同步时，启动前无需登录 QQ，可以先启动代理再登录；但捕获 Code 前必须稳定确认 QQ，否则 Code 只保留在内存中且不会请求服务器。若获取期间发生切号，同样停止同步，从而避免创建默认的无 QQ 号账号。
- 若本地检测不可用，关闭自动同步后仍可只获取 Code，但工具不会自动在服务器创建账号。
- 农场 WebSocket 请求本身不包含 UIN、昵称或头像，身份信息不是从 Code 中解码得到的。

## Token 说明

- 请使用 `qq-farm-bot` 网页侧边栏中的当前登录 Token。
- 工具通过 `x-admin-token` 请求头访问服务器。
- Token 保存在 Windows 凭据管理器，不会写入 `settings.json`。
- `qq-farm-bot` 服务器重启或 Token 失效时，需要从网页重新登录并复制新 Token。

## 配置与运行文件位置

- `settings.json`、`system-proxy-backup.json`、`temporary-ca.cer` 和 `traffic-diagnostics.log` 均存放在应用 EXE 所在目录，不使用 `%APPDATA%` 默认目录。
- 首次运行新版时，会自动把旧版本位于 `%APPDATA%\io.github.ccpopy.qqfarmcodehelper` 的上述文件迁移到 EXE 所在目录。
- 便携版可直接放到任意有写入权限的目录；配置和恢复文件会跟随 EXE，移动时可一并迁移。
- 安装版同样将运行文件放在安装目录。若自行选择安装位置，请确保当前 Windows 用户对该目录具有写入权限。
- 服务器 Token 仍由 Windows 凭据管理器加密保存，不会以明文文件放在应用目录。

## GitHub Release

推送 `v*` 格式的 Tag（例如 `v0.1.2`）会触发 GitHub Actions，在对应 Release 中自动提供：

- `QQFarmCodeHelper-<tag>-portable.exe`：单文件便携版；
- `QQFarmCodeHelper-<tag>-setup.exe`：Windows NSIS 安装包。

## 安全边界

- 代理只监听 `127.0.0.1`，不向局域网开放。
- 非 QQ 农场 HTTPS 连接只做 TCP 直通，不解密内容。
- 只有 `gate-obt.nqf.qq.com/prod/ws` 可以触发 Code 提取。
- Code 不写日志、不写配置文件。
- 正常退出会恢复原系统代理并移除临时根证书。
- 异常崩溃时会保留代理备份；下次启动会先自动恢复，也可点击「清理代理与证书」。

## 开发与构建

需要 Windows 10/11、Microsoft Edge WebView2 Runtime 和 Rust stable MSVC toolchain。

```powershell
# 开发运行
.\scripts\dev.ps1

# 构建 release EXE
.\scripts\build.ps1
```

产物位于 `release\QQFarmCodeHelper.exe`。Tauri 不内置 WebView2 Runtime，因此程序本体保持较小。

## 测试

```powershell
cd src-tauri
cargo test
cargo clippy --all-targets -- -D warnings
```

代理集成测试会在本机启动随机端口，通过模拟 HTTPS 代理请求验证 Code 提取和 451 阻断，不会连接腾讯上游。
