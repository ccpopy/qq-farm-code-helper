import Foundation

/// The shared, cross-process capture state. The tunnel extension is the only writer while a
/// round is collecting; the app writes when the user consumes or clears a result.
struct CaptureSession: Codable, Equatable, Sendable {
    var generation: Int
    var platform: CapturePlatform
    var code: String
    var ownGid: String
    var gids: [String]
    var createdAt: Date
    var friendsObserved: Bool
    var collecting: Bool
    var published: Bool
    var consumedGids: [String]
    var expiredCode: Bool
    var updatedAt: Date

    static let codeLifetime: TimeInterval = 300

    var hasPending: Bool { !code.isEmpty || !gids.isEmpty }
    var isCodeExpired: Bool {
        expiredCode || (!code.isEmpty && Date().timeIntervalSince(createdAt) >= Self.codeLifetime)
    }

    func summary(capturing: Bool) -> String {
        let suffix = capturing ? " · VPN 保持运行" : ""
        let expired = isCodeExpired
        if !gids.isEmpty {
            let prefix = expired ? "Code 已过期 · " : (code.isEmpty ? "" : "Code 已取得 · ")
            return prefix + "已读取 \(gids.count) 位好友，可立即同步" + suffix
        }
        if !code.isEmpty {
            return "Code 已取得，可立即同步" + (collecting ? " · 等待好友回包" : "") + suffix
        }
        if !consumedGids.isEmpty {
            return (expired ? "Code 已过期 · " : "") + "已同步当前好友" + (collecting ? " · 继续接收新好友" : "") + suffix
        }
        if friendsObserved {
            return (expired ? "Code 已过期 · " : "") + "已收到好友回包 · 暂无待同步好友" + suffix
        }
        if expired {
            return "Code 已过期，请清除后重新获取" + (collecting ? " · 仍可接收好友" : "") + suffix
        }
        return (collecting ? "农场已连接 · 等待好友回包" : "当前结果已同步") + suffix
    }
}
