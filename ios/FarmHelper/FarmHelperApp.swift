import Foundation
import SwiftUI
import UserNotifications
import UIKit

@main
@MainActor
struct FarmHelperApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(model)
                .onAppear {
                    model.start()
                    if let info = AppDelegate.pendingNotification {
                        AppDelegate.pendingNotification = nil
                        model.handleNotificationTap(userInfo: info)
                    }
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)) { _ in
                    model.applicationBecameActive()
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.didEnterBackgroundNotification)) { _ in
                    BackgroundRefresh.shared.schedule()
                }
                .onReceive(NotificationCenter.default.publisher(for: AppDelegate.notificationTapped)) { note in
                    AppDelegate.pendingNotification = nil
                    model.handleNotificationTap(userInfo: note.userInfo ?? [:])
                }
        }
    }
}
