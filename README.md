# QQ农场 Code 获取助手

一个面向 Windows QQ 客户端的小型 Rust/Tauri 应用。它会在本机临时启用定向代理，仅解密 `gate-obt.nqf.qq.com:443`，从 QQ 农场的 `/prod/ws` 登录请求中提取一次性 `code`，并在请求到达腾讯服务器前阻断它。

获取后可以：

- 自动调用 `qq-farm-bot` 的 `POST /api/accounts` 新增 QQ 账号；
- 或者仅保存在内存中，由用户点击复制。

## 使用流程

1. 在服务器设置中填写 `qq-farm-bot` 管理页面地址。
2. 在 `qq-farm-bot` 左侧边栏底部复制当前登录 Token，填入工具。
3. 点击「测试连接」，确认 `/api/user/me` 可以正常返回当前用户。
4. 保存设置并点击「启动获取」。
5. **完全退出 Windows QQ**，重新打开 QQ，然后进入 QQ 农场。
6. 工具捕获 Code 后会立即恢复系统代理、移除临时根证书，再同步到服务器。

QQ 农场窗口当次出现网络错误是预期现象：官方的 WebSocket 登录请求被本地工具主动阻断，避免一次性 Code 被官方客户端先消耗。

## Token 说明

- 请使用 `qq-farm-bot` 网页侧边栏中的当前登录 Token。
- 工具通过 `x-admin-token` 请求头访问服务器。
- Token 保存在 Windows 凭据管理器，不会写入 `settings.json`。
- `qq-farm-bot` 服务器重启或 Token 失效时，需要从网页重新登录并复制新 Token。

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
