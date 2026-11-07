import Foundation

/// Long-poll monitor that runs inside the tunnel extension. It is the only place where account
/// alerts can keep working while the app is suspended, which mirrors the Android foreground service.
final class ExtensionMonitor: @unchecked Sendable {
    private let lock = NSLock()
    private var coordinator: Task<Void, Never>?
    private var workers: [String: Task<Void, Never>] = [:]
    private var paused: [String: String] = [:]
    private var active = false
    private var isActive: (() -> Bool)?

    func start(isActive: @escaping () -> Bool) {
        stop()
        self.isActive = isActive
        active = true
        coordinator = Task.detached { [weak self] in
            guard let self else { return }
            while self.active, isActive() {
                self.reconcile()
                try? await Task.sleep(nanoseconds: 5_000_000_000)
            }
        }
    }

    func stop() {
        active = false
        lock.lock()
        let coordinator = self.coordinator
        let workers = Array(self.workers.values)
        self.coordinator = nil
        self.workers.removeAll()
        self.paused.removeAll()
        lock.unlock()
        coordinator?.cancel()
        workers.forEach { $0.cancel() }
    }

    private func reconcile() {
        let settings = SharedSettings.shared
        let value = settings.load()
        let servers = value.monitorEnabled
            ? settings.readyServers().filter { !$0.monitoredIds.isEmpty }
            : []
        let wanted = Set(servers.map(\.id))

        lock.lock()
        for server in servers {
            if let pausedConnection = paused[server.id], pausedConnection != server.connectionId {
                paused.removeValue(forKey: server.id)
            }
        }
        for (id, task) in workers where !wanted.contains(id) {
            task.cancel()
            workers.removeValue(forKey: id)
        }
        for server in servers where workers[server.id] == nil {
            if paused[server.id] == server.connectionId { continue }
            let task = Task.detached { [weak self] in
                guard let self else { return }
                await self.run(serverId: server.id)
            }
            workers[server.id] = task
        }
        lock.unlock()
    }

    private func run(serverId: String) async {
        defer {
            lock.lock()
            workers.removeValue(forKey: serverId)
            lock.unlock()
        }
        let transitions = AccountTransitions()
        var relayUnavailable = false
        var cursor = ""
        var delivered: [String: String] = [:]
        var relaySelection = Set<String>()
        var failures = 0
        var announcedRelay = false

        while !Task.isCancelled, active, isActive?() == true {
            let settings = SharedSettings.shared
            guard let server = settings.server(id: serverId),
                  server.ready(token: settings.token(for: serverId)),
                  !server.monitoredIds.isEmpty else { break }
            let selected = Set(server.monitoredIds)
            transitions.retain(selected)

            do {
                let client = try APIClient(server: server)
                if !relayUnavailable {
                    if relaySelection != selected {
                        relaySelection = selected
                        cursor = ""
                        delivered = delivered.filter { selected.contains($0.key) }
                    }
                    do {
                        try await relayOnce(client: client, server: server, selected: selected, cursor: &cursor, delivered: &delivered)
                        failures = 0
                        if !announcedRelay {
                            announcedRelay = true
                            SharedLog.append("通知", "服务器中转已连接，等待账号下线事件", source: .tunnel)
                        }
                        continue
                    } catch let error as APIError where error == .relayUnsupported {
                        relayUnavailable = true
                        SharedLog.append("通知", "服务器尚未提供手机中转，使用原有定时检查", source: .tunnel)
                    }
                }

                try await fallbackOnce(client: client, server: server, selected: selected, transitions: transitions)
                failures = 0
                let seconds = max(1, settings.load().pollSeconds)
                try await Task.sleep(nanoseconds: UInt64(seconds) * 1_000_000_000)
            } catch is CancellationError {
                break
            } catch let error as APIError where error == .auth {
                AlertNotifier.serverAuth(server)
                SharedLog.append("通知", "服务器认证失败，已暂停该服务器的检查；请更新管理 Token", source: .tunnel)
                lock.lock()
                paused[serverId] = server.connectionId
                lock.unlock()
                break
            } catch {
                if Task.isCancelled { break }
                transitions.interrupted()
                failures = min(5, failures + 1)
                if failures == 1 {
                    SharedLog.append("通知", "检查失败，已退避重试；不会把网络故障当作账号下线", source: .tunnel)
                }
                let interval = max(1, settings.load().pollSeconds)
                let delay = min(900, interval * ((1 << failures) - 1))
                try? await Task.sleep(nanoseconds: UInt64(delay) * 1_000_000_000)
            }
        }
    }

    private func relayOnce(
        client: APIClient,
        server: ServerProfile,
        selected: Set<String>,
        cursor: inout String,
        delivered: inout [String: String]
    ) async throws {
        let batch = try await client.offlineEvents(cursor: cursor, selected: selected)
        guard client.isCurrent else { return }
        var index: [String: Account] = [:]
        if !batch.events.isEmpty {
            let list = try await client.accounts()
            for account in list { index[account.id] = account }
        }
        for event in batch.events {
            guard let account = index[event.accountId],
                  account.uuid == event.accountUuid,
                  account.platform == event.platform,
                  delivered[account.id] != event.eventId else { continue }
            if try await client.connected(account) { continue }
            AlertNotifier.alert(
                title: account.name + " 需要重新登录",
                message: server.label + " · 服务器检测到账号掉线。点此打开对应账号，重新扫码或获取 Code 后同步。",
                server: server,
                accountId: account.id,
                accountUuid: account.uuid
            )
            delivered[account.id] = event.eventId
            SharedLog.append("通知", "已收到服务器下线事件，通知绑定对应账号", source: .tunnel)
        }
        cursor = batch.cursor
    }

    private func fallbackOnce(
        client: APIClient,
        server: ServerProfile,
        selected: Set<String>,
        transitions: AccountTransitions
    ) async throws {
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
        for id in selected {
            let account = index[id]
            guard transitions.observe(id, online: observations[id] == true) else { continue }
            AlertNotifier.alert(
                title: (account?.name ?? "服务器账号") + " 已离线",
                message: server.label + " · " + (account == nil
                    ? "账号已从服务器列表移除，请在管理页面确认。"
                    : "连续两次未连接。可能是登录失效、手动停止或断线；点此重新获取 Code。"),
                server: server,
                accountId: id,
                accountUuid: account?.uuid ?? ""
            )
            SharedLog.append("通知", "已提醒一个离线账号；通知已绑定服务器账号 ID", source: .tunnel)
        }
    }
}
