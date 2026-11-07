import Foundation
import UserNotifications

/// Local notifications are used by both the app and the tunnel extension. Tapping one pins the
/// server id, account id and account uuid so the app can reject a stale notification.
enum AlertNotifier {
    static let accountKey = "serverId"
    static let accountIdKey = "accountId"
    static let accountUUIDKey = "accountUuid"
    static let connectionKey = "connectionId"

    static func requestAuthorization() async -> Bool {
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        if settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional {
            return true
        }
        return (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
    }

    static func alert(title: String, message: String, server: ServerProfile, accountId: String, accountUuid: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = message
        content.sound = .default
        content.userInfo = [
            accountKey: server.id,
            accountIdKey: accountId,
            accountUUIDKey: accountUuid,
            connectionKey: server.connectionId,
        ]
        let identifier = "account/\(server.id)/\(accountId)"
        let request = UNNotificationRequest(identifier: identifier, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request) { error in
            if let error {
                SharedLog.append("通知", "本地通知未能投递：\(error.localizedDescription)", source: .app)
            }
        }
    }

    static func serverAuth(_ server: ServerProfile) {
        let content = UNMutableNotificationContent()
        content.title = "\(server.label) 的 Token 需要更新"
        content.body = "该服务器的掉线检查已停止；不代表游戏登录失效。点击更新服务器设置。"
        content.sound = .default
        content.userInfo = [accountKey: server.id]
        let request = UNNotificationRequest(
            identifier: "server-auth/\(server.id)",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    static func captureReady(title: String, message: String, platform: CapturePlatform) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = message
        content.sound = .default
        content.userInfo = ["kind": "capture", "platform": platform.rawValue]
        let request = UNNotificationRequest(identifier: "capture-ready", content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }
}
