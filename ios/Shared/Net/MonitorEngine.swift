import Foundation

/// One-shot account check used by the app's background refresh task. The long-poll relay lives
/// in the tunnel extension; this path is the "old server" fallback and runs opportunistically.
enum MonitorEngine {
    private struct State: Codable {
        var offlineCounts: [String: Int] = [:]
        var notified: Set<String> = []
        var lastChecked: Date?
    }

    private static let stateKey = "monitor.bg.state.v1"
    private static let lock = NSLock()

    static func checkAllOnce(settings: SharedSettings = .shared) async {
        guard settings.load().monitorEnabled else { return }
        let servers = settings.readyServers().filter { !$0.monitoredIds.isEmpty }
        for server in servers {
            await check(server: server, settings: settings)
        }
    }

    private static func check(server: ServerProfile, settings: SharedSettings) async {
        let selected = Set(server.monitoredIds)
        guard !selected.isEmpty else { return }
        do {
            let client = try APIClient(server: server, settings: settings)
            let accounts = try await client.accounts()
            var index: [String: Account] = [:]
            for account in accounts { index[account.id] = account }

            var observations: [String: Bool] = [:]
            for id in selected {
                guard let account = index[id] else {
                    observations[id] = false
                    continue
                }
                observations[id] = try await client.connected(account)
            }
            guard client.isCurrent else { return }

            var state = loadState()
            var failures = 0
            for id in selected {
                let key = "\(server.id)/\(id)"
                let account = index[id]
                if observations[id] == true {
                    state.offlineCounts.removeValue(forKey: key)
                    state.notified.remove(key)
                    continue
                }
                let count = min(2, (state.offlineCounts[key] ?? 0) + 1)
                state.offlineCounts[key] = count
                guard count >= 2, !state.notified.contains(key) else { continue }
                state.notified.insert(key)
                failures += 1
                AlertNotifier.alert(
                    title: (account?.name ?? "服务器账号") + " 已离线",
                    message: server.label + " · " + (account == nil
                        ? "账号已从服务器列表移除，请在管理页面确认。"
                        : "连续两次未连接。可能是登录失效、手动停止或断线；点此重新获取 Code。"),
                    server: server,
                    accountId: id,
                    accountUuid: account?.uuid ?? ""
                )
            }
            state.lastChecked = Date()
            saveState(state)
            if failures > 0 {
                SharedLog.append("通知", "已提醒 \(failures) 个离线账号；通知已绑定服务器账号 ID", source: .app)
            }
        } catch let error as APIError where error == .auth {
            AlertNotifier.serverAuth(server)
            SharedLog.append("通知", "服务器认证失败，已暂停该服务器的检查；请更新管理 Token", source: .app)
            clearServerState(server.id)
        } catch {
            // A network error must not become an offline event.
            interruptServer(server.id)
            SharedLog.append("通知", "检查失败，已退避重试；不会把网络故障当作账号下线", source: .app)
        }
    }

    private static func loadState() -> State {
        lock.lock()
        defer { lock.unlock() }
        guard let data = AppGroup.defaults.data(forKey: stateKey),
              let state = try? JSONDecoder().decode(State.self, from: data) else { return State() }
        return state
    }

    private static func saveState(_ state: State) {
        lock.lock()
        defer { lock.unlock() }
        if let data = try? JSONEncoder().encode(state) {
            AppGroup.defaults.set(data, forKey: stateKey)
        }
    }

    private static func interruptServer(_ serverId: String) {
        var state = loadState()
        state.offlineCounts = state.offlineCounts.filter { !$0.key.hasPrefix(serverId + "/") }
        saveState(state)
    }

    private static func clearServerState(_ serverId: String) {
        var state = loadState()
        state.offlineCounts = state.offlineCounts.filter { !$0.key.hasPrefix(serverId + "/") }
        state.notified = state.notified.filter { !$0.hasPrefix(serverId + "/") }
        saveState(state)
    }
}
