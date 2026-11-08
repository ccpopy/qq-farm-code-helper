import Foundation

/// A network error never becomes an offline event. Two observations debounce reconnects.
final class AccountTransitions {
    private var offlineCount: [String: Int] = [:]
    private var notified = Set<String>()

    func observe(_ id: String, online: Bool) -> Bool {
        if online {
            offlineCount.removeValue(forKey: id)
            notified.remove(id)
            return false
        }
        let count = min(2, (offlineCount[id] ?? 0) + 1)
        offlineCount[id] = count
        // Also alert for accounts already offline when monitoring starts (after confirmation).
        guard count >= 2, !notified.contains(id) else { return false }
        notified.insert(id)
        return true
    }

    /// Failed checks break consecutiveness, but must not re-alert an already notified account.
    func interrupted() {
        offlineCount.removeAll()
    }

    func retain(_ ids: Set<String>) {
        offlineCount = offlineCount.filter { ids.contains($0.key) }
        notified = notified.filter { ids.contains($0) }
    }
}
