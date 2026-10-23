# Android 交付约定

- 用户要求只交付一个应用：名称固定“农场助手”，包名固定 `io.github.ccpopy.farmhelper`。
  不追加测试、诊断等名称或 applicationIdSuffix，不用旧的 `outputs/android/qa/*.init.gradle` 生成并存包。
- Android Debug / Release 必须使用同一份长期签名私钥。默认位置为
  `%USERPROFILE%/.android/farm-helper/farm-helper.jks`，各 checkout / agent 共用。
  本机文件是现有安装签名密钥的保留副本，不得重新生成或替换。口令与别名配置在同目录 `signing.properties`。
- 预期签名证书 SHA-256：`cf51c2ee1ed4870e98751894f41cb5406b7f2cd29645df2fa5186f829235c749`。
  换机器须安全迁移同一私钥；只有指纹无法生成同一签名。密钥缺失时停止打包，不回退到系统自动生成的 debug key。
  密钥与口令不得提交 Git；环境变量配置见 `android/README.md`。
- 通过 `scripts/build-android.ps1` 构建；脚本检查包名、应用名称与签名，统一交付
  `outputs/android/QQFarmCodeHelper.apk`。安装使用 `adb install -r` 保留数据，不通过卸载解决签名冲突。
- 覆盖升级必须继续使用用户已安装的抓包 CA：保留私有目录中的 `local-ca.cer` 与 Android Keystore
  别名 `farmhelper.local-ca.rsa.v1`，不得因版本变化自动重建、删除或替换。应用签名与抓包 CA 是两套身份。
  调整证书存储时须兼容现有数据；更新验收比较 CA 指纹，并确认系统仍安装同一张证书。
- 不为测试随意更改版本或发布 Release。不要在日志、测试输出或 Git 中留下真实 Code、Token、好友 GID 或抓包材料。
- `qq-farm-bot` 后台改动在用户指定的 main 分支处理；不要把后台源码混入此 Android 仓库。
