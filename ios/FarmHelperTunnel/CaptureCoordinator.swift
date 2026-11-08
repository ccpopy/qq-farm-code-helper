import Foundation

/// Owns the current round for one tunnel session. `generation` is read when a TCP connection is
/// accepted; clearing or replacing the result in the app invalidates that connection's claim.
final class CaptureCoordinator: @unchecked Sendable {
    let platform: CapturePlatform
    let friendCapture: Bool
    let passthrough: Bool

    private let store = CaptureStore.shared
    private let lock = NSLock()
    private var notifiedGeneration = -1

    init(platform: CapturePlatform, friendCapture: Bool, passthrough: Bool) {
        self.platform = platform
        self.friendCapture = friendCapture
        self.passthrough = passthrough
    }

    @discardableResult
    func begin() -> Int {
        let generation = store.beginRound(platform: platform)
        SharedLog.append(
            "抓取",
            "开始新一轮获取；仅解析 \(UrlPolicy.farmHost) 的 TLS，其他流量原样透传",
            source: .tunnel
        )
        return generation
    }

    /// Publish the login Code. For WeChat friend-only mode the Code is forwarded upstream and
    /// never retained, matching the Android behavior.
    func publish(code: String, generation: Int) -> Bool {
        let storedCode = (platform == .wx && friendCapture) ? "" : code
        guard store.publish(code: storedCode, generation: generation, platform: platform) else {
            return false
        }
        SharedLog.append("抓取", "已取得可同步结果；可立即推送，VPN 将继续接收好友，清除结果可换号获取", source: .tunnel)
        notifyIfNeeded(friendsOnly: storedCode.isEmpty)
        return true
    }

    func merge(generation: Int, ownGid: String, gids: [String], friendsObserved: Bool, complete: Bool) {
        let changed = store.merge(
            generation: generation,
            platform: platform,
            ownGid: ownGid,
            gids: gids,
            friendsObserved: friendsObserved,
            complete: complete
        )
        if changed, friendsObserved {
            notifyIfNeeded(friendsOnly: false)
        }
    }

    func finish(generation: Int) {
        store.finish(generation: generation)
        if let session = store.current(), session.generation == generation,
           session.collecting == false, !session.friendsObserved {
            SharedLog.append("抓取", "尚未观察到成功的好友回包；不会伪报好友同步成功", source: .tunnel)
        }
    }

    private func notifyIfNeeded(friendsOnly: Bool) {
        let generation = store.current()?.generation ?? -1
        lock.lock()
        guard generation != notifiedGeneration else {
            lock.unlock()
            return
        }
        notifiedGeneration = generation
        lock.unlock()
        AlertNotifier.captureReady(
            title: friendsOnly ? "好友已读取" : "农场凭据已获取",
            message: "可立即确认账号并推送；VPN 保持运行。",
            platform: platform
        )
    }
}
