# iOS 更新日志

## ios-v0.1.3

- 首个 iOS 版本，按 Android 0.1.3 的功能移植：多服务器账号、QQ / 微信 VPN 抓取、微信扫码登录、推送与新增/覆盖确认、掉线提醒、证书管理、日志和更新检查。
- 使用 `NEPacketTunnelProvider` + hev-socks5-tunnel 2.19.1；只解密 `gate-obt.nqf.qq.com` 的 TLS，其他流量原样透传。
- 本地 CA 通过 `.mobileconfig` 描述文件安装；叶子证书镜像上游证书的 Subject、DNS SAN 和有效期。
- Code 与好友 GID 仅用于本机结果与用户确认后的服务器写入；跨进程结果快照使用 Keychain 中的设备专属密钥加密。
- GitHub Actions：推送 / PR 编译未签名 IPA；推送 `ios-vX.Y.Z` 标签编译并发布 Release；配置签名密钥后同时产出已签名 IPA。

