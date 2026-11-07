import BackgroundTasks
import Foundation
import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    static let notificationTapped = Notification.Name("FarmHelperNotificationTapped")
    static var pendingNotification: [AnyHashable: Any]?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        BGTaskScheduler.shared.register(forTaskWithIdentifier: BackgroundRefresh.identifier, using: nil) { task in
            guard let refresh = task as? BGAppRefreshTask else {
                task.setTaskCompleted(success: false)
                return
            }
            BackgroundRefresh.shared.handle(refresh)
        }
        return true
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .list])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let info = response.notification.request.content.userInfo
        Self.pendingNotification = info
        NotificationCenter.default.post(name: Self.notificationTapped, object: nil, userInfo: info)
        completionHandler()
    }
}

final class BackgroundRefresh: @unchecked Sendable {
    static let shared = BackgroundRefresh()
    static let identifier = "io.github.ccpopy.farmhelper.refresh"

    var handler: (() async -> Void)?

    private init() {}

    func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: Self.identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    func handle(_ task: BGAppRefreshTask) {
        schedule()
        let work = Task { [handler] in
            if let handler {
                await handler()
            } else if await TunnelManager.shared.refreshStatus() != .connected {
                await MonitorEngine.checkAllOnce()
            }
        }
        task.expirationHandler = { work.cancel() }
        Task {
            await work.value
            task.setTaskCompleted(success: !work.isCancelled)
        }
    }
}
