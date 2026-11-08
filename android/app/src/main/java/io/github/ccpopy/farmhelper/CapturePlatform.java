package io.github.ccpopy.farmhelper;

/** A capture belongs to one app and one server platform for its entire lifetime. */
public enum CapturePlatform {
    QQ("qq", "QQ", "com.tencent.mobileqq"),
    WECHAT("wx", "微信", "com.tencent.mm");

    public final String id, label, packageName;
    CapturePlatform(String id, String label, String packageName) {
        this.id = id; this.label = label; this.packageName = packageName;
    }
    public static CapturePlatform fromId(String id) {
        for (CapturePlatform platform : values()) if (platform.id.equals(id)) return platform;
        throw new IllegalArgumentException("暂不支持此账号平台");
    }
    public String compatibilityHint() {
        return this == WECHAT ? "微信获取为实验性功能：证书兼容性及 Code、好友 GID 获取尚未验证成功。能进入农场不代表已取得凭据。" : "";
    }
}
