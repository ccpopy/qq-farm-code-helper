import Foundation

/// One capture belongs to one app and one server platform for its entire lifetime.
enum CapturePlatform: String, Codable, CaseIterable, Identifiable, Sendable {
    case qq
    case wx

    var id: String { rawValue }
    var label: String { self == .qq ? "QQ" : "微信" }
    var bundleIdentifier: String { self == .qq ? "com.tencent.mobileqq" : "com.tencent.mm" }
    var urlScheme: String { self == .qq ? "mqq://" : "weixin://" }

    var compatibilityHint: String {
        self == .wx
            ? "微信获取为实验性功能：证书兼容性及 Code、好友 GID 获取尚未验证成功。能进入农场不代表已取得凭据。"
            : ""
    }
}
