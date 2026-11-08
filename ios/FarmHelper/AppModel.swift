import Combine
import Foundation
import NetworkExtension
import UIKit

struct PushDestination: Identifiable {
    let id = UUID()
    let capture: CaptureSession
    let groups: [ServerGroup]
    let code: Bool
    let friends: Bool
    let recommended: ServerDirectory.Choice?
    let detected: Bool
}

struct WxLoginPreset {
    let serverId: String
    let targetId: String
}

struct ReloginPrompt: Identifiable {
    let id = UUID()
    let serverId: String
    let accountId: String
    let accountName: String
    let platform: String
}

@MainActor
final class AppModel: ObservableObject {
    @Published var settings: AppSettings
    @Published var capture: CaptureSession?
    @Published var captureStatus = "暂无获取结果"
    @Published var logs = "暂无日志"
    @Published var groups: [ServerGroup] = []
    @Published var online: [String: Bool] = [:]
    @Published var serverErrors: [String: String] = [:]
    @Published var loadingAccounts = false
    @Published var busy = false
    @Published var toast: String?
    @Published var vpnSetupError: String?
    @Published var tunnelStatus: TunnelStatus?
    @Published var vpnState: NEVPNStatus = .invalid
    @Published var certExists = false
    @Published var certTrusted: Bool?
    @Published var selectedTab = 0
    @Published var pushDestination: PushDestination?
    @Published var showWxLogin = false
    @Published var wxLoginPreset: WxLoginPreset?
    @Published var showManualImport = false
    @Published var showCertificate = false
    @Published var reloginPrompt: ReloginPrompt?
    @Published var updateInfo: UpdateInfo?
    @Published var showUpdate = false

    private let store = SharedSettings.shared
    private let captureStore = CaptureStore.shared
    private var pollTask: Task<Void, Never>?
    private var started = false
    private var accountLoadID = UUID()
    private var lastAccountsRefresh = Date.distantPast
    private var lastToast = ""
    private var handledCapture = -1
    private var trustCheckRunning = false

    init() {
        settings = SharedSettings.shared.load()
        capture = CaptureStore.shared.current()
        certExists = CertificateAuthority.shared.exists
        BackgroundRefresh.shared.handler = { [weak self] in
            guard let self else { return }
            await self.runBackgroundRefresh()
        }
    }

    // MARK: - Lifecycle

    func start() {
        guard !started else { return }
        started = true
        refreshShared()
        NotificationCenter.default.addObserver(
            forName: .NEVPNStatusDidChange, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                self?.vpnState = TunnelManager.shared.status
                self?.refreshShared()
            }
        }
        DarwinNotifier.shared.add(AppGroup.captureChanged) { [weak self] in
            Task { @MainActor in self?.refreshShared() }
        }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard let self else { return }
                self.refreshShared()
                if self.vpnState == .connected { self.tunnelStatus = TunnelStatusStore.load() }
            }
        }
        Task {
            vpnState = await TunnelManager.shared.refreshStatus()
            refreshShared()
        }
        Task {
            let granted = await AlertNotifier.requestAuthorization()
            if !granted, settings.monitorEnabled {
                updateSettings { $0.monitorEnabled = false }
                syncMonitor()
            }
        }
        refreshAccounts()
        refreshCertificateTrust()
        pingServers()
        BackgroundRefresh.shared.schedule()
    }

    func applicationBecameActive() {
        refreshShared()
        refreshCertificateTrust()
        if Date().timeIntervalSince(lastAccountsRefresh) > 30 {
            refreshAccounts()
        }
    }

    // MARK: - Shared state

    func refreshShared() {
        let stored = store.load()
        if stored != settings { settings = stored }
        let current = captureStore.current()
        if current != capture { capture = current }
        let status = captureStore.statusText(
            capturing: vpnState == .connected || vpnState == .connecting,
            platform: settings.capturePlatform
        )
        if status != captureStatus { captureStatus = status }
        let newLogs = SharedLog.text()
        let text = newLogs.isEmpty ? "暂无日志" : newLogs
        if text != logs { logs = text }
        let tunnel = TunnelStatusStore.load()
        if tunnel != tunnelStatus { tunnelStatus = tunnel }
        let state = TunnelManager.shared.status
        if state != vpnState { vpnState = state }

        if let current, current.hasPending, current.generation != handledCapture,
           settings.autoPush, !busy, pushDestination == nil,
           UIApplication.shared.applicationState == .active {
            handledCapture = current.generation
            preparePush(automatic: true)
        }
    }

    // MARK: - Settings

    func updateSettings(_ body: (inout AppSettings) -> Void) {
        var value = store.load()
        body(&value)
        store.save(value)
        settings = value
    }

    func saveServer(existing: ServerProfile?, name: String, address: String, token: String, allowHTTP: Bool) async throws -> String {
        let profile = try store.saveServer(
            existingId: existing?.id, name: name, address: address, token: token, allowHTTP: allowHTTP
        )
        settings = store.load()
        var status = ""
        do {
            _ = try await APIClient(server: profile).ping()
        } catch {
            status = ServerDirectory.reason(error)
        }
        if status.isEmpty {
            serverErrors.removeValue(forKey: profile.id)
            showToast("已连接 \(profile.label)")
        } else {
            serverErrors[profile.id] = status
            showToast("已保存，但\(status)。可以稍后再试。")
        }
        refreshAccounts()
        if vpnState == .connected || vpnState == .connecting {
            TunnelManager.shared.send("reload")
        }
        return status
    }

    func deleteServer(_ server: ServerProfile) {
        store.removeServer(id: server.id)
        settings = store.load()
        groups.removeAll { $0.id == server.id }
        online = online.filter { !$0.key.hasPrefix(server.id + "/") }
        serverErrors.removeValue(forKey: server.id)
        showToast("已删除服务器「\(server.label)」")
        refreshAccounts()
        if vpnState == .connected || vpnState == .connecting {
            TunnelManager.shared.send("reload")
        }
    }

    func pingServers() {
        let servers = settings.servers
        Task {
            await withTaskGroup(of: (String, String).self) { group in
                for server in servers {
                    guard server.ready(token: store.token(for: server.id)) else { continue }
                    group.addTask {
                        do {
                            _ = try await APIClient(server: server).ping()
                            return (server.id, "")
                        } catch {
                            return (server.id, ServerDirectory.reason(error))
                        }
                    }
                }
                for await (id, status) in group {
                    if status.isEmpty { serverErrors.removeValue(forKey: id) } else { serverErrors[id] = status }
                }
            }
        }
    }

    // MARK: - Accounts

    func refreshAccounts() {
        guard !settings.servers.isEmpty else {
            groups = []
            loadingAccounts = false
            return
        }
        lastAccountsRefresh = Date()
        loadingAccounts = true
        let loadID = UUID()
        accountLoadID = loadID
        Task {
            let result = await ServerDirectory.load(platform: nil, settings: store)
            guard accountLoadID == loadID else { return }
            groups = result
            loadingAccounts = false
            for group in result {
                if let error = group.error { serverErrors[group.id] = error } else { serverErrors.removeValue(forKey: group.id) }
            }
            await checkOnline(result, loadID: loadID)
        }
    }

    func retryAccounts(_ server: ServerProfile) {
        let loadID = UUID()
        accountLoadID = loadID
        loadingAccounts = true
        Task {
            let result = await ServerDirectory.read(server: server, platform: nil, settings: store)
            guard accountLoadID == loadID else { return }
            if let index = groups.firstIndex(where: { $0.id == server.id }) {
                groups[index] = result
            } else {
                groups.append(result)
            }
            loadingAccounts = false
            if let error = result.error { serverErrors[server.id] = error } else { serverErrors.removeValue(forKey: server.id) }
            await checkOnline([result], loadID: loadID)
        }
    }

    private func checkOnline(_ groups: [ServerGroup], loadID: UUID) async {
        await withTaskGroup(of: (String, Bool).self) { group in
            for serverGroup in groups where serverGroup.ok {
                guard let client = serverGroup.client else { continue }
                for account in serverGroup.accounts where account.running {
                    let key = onlineKey(serverGroup.server, account)
                    group.addTask {
                        let state = (try? await client.connected(account)) ?? false
                        return (key, state)
                    }
                }
            }
            for await (key, state) in group {
                guard accountLoadID == loadID else { return }
                online[key] = state
            }
        }
    }

    private func onlineKey(_ server: ServerProfile, _ account: Account) -> String {
        "\(server.id)/\(server.connectionId)/\(account.id)"
    }

    private func onlineKey(serverId: String, connectionId: String, accountId: String) -> String {
        "\(serverId)/\(connectionId)/\(accountId)"
    }

    func onlineState(server: ServerProfile, account: Account) -> Bool? {
        online[onlineKey(server, account)]
    }

    func toggleWatch(server: ServerProfile, account: Account) {
        guard let current = settings.server(id: server.id), current.connectionId == server.connectionId else {
            showToast("服务器配置已变化，请刷新账号")
            return
        }
        var ids = Set(current.monitoredIds)
        if ids.contains(account.id) { ids.remove(account.id) } else { ids.insert(account.id) }
        store.monitor(serverId: server.id, ids: ids)
        settings = store.load()
        syncMonitor()
    }

    func selectAll(server: ServerProfile, group: ServerGroup) {
        guard group.current else {
            showToast("服务器配置已变化，请刷新账号")
            return
        }
        let watched = Set(server.monitoredIds)
        let all = group.accounts.allSatisfy { watched.contains($0.id) }
        if all {
            store.monitor(serverId: server.id, ids: [])
        } else {
            store.monitor(serverId: server.id, ids: Set(group.accounts.map(\.id)))
        }
        settings = store.load()
        syncMonitor()
    }

    func setMonitorEnabled(_ enabled: Bool) {
        updateSettings { $0.monitorEnabled = enabled }
        if enabled {
            Task {
                let granted = await AlertNotifier.requestAuthorization()
                if !granted {
                    updateSettings { $0.monitorEnabled = false }
                    syncMonitor()
                    showToast("未允许通知，掉线提醒已关闭")
                }
            }
        }
        syncMonitor()
        if enabled, !store.anyMonitored() {
            showToast("掉线提醒已开启 · 未选择账号")
        }
    }

    private func syncMonitor() {
        if vpnState == .connected || vpnState == .connecting {
            TunnelManager.shared.send("reload")
        }
        BackgroundRefresh.shared.schedule()
    }

    // MARK: - Capture

    func switchPlatform(_ platform: CapturePlatform) {
        guard platform != settings.capturePlatform else { return }
        guard vpnState != .connected, vpnState != .connecting else {
            showToast("请先停止获取，再切换应用")
            return
        }
        updateSettings { value in
            value.capturePlatform = platform
            for index in value.servers.indices {
                value.servers[index].targetId = ""
                value.servers[index].targetName = "未选择账号"
                value.servers[index].targetGid = ""
                value.servers[index].targetPlatform = "qq"
            }
        }
        captureStore.clear()
        capture = nil
        refreshShared()
        showToast("已切换到 \(platform.label)")
    }

    func startTunnel() {
        guard !settings.servers.isEmpty else {
            showToast("请先添加服务器")
            selectedTab = 3
            return
        }
        guard vpnState != .connected, vpnState != .connecting else { return }
        if !settings.passthrough {
            guard certExists else {
                showToast("首次使用需要生成并安装证书")
                showCertificate = true
                return
            }
            if certTrusted == false {
                showToast("证书尚未被系统信任；若进入农场后握手失败，请先安装并打开完全信任")
            }
        }
        Task {
            do {
                try await TunnelManager.shared.start(settings: settings)
                showToast("正在启动 VPN")
                BackgroundRefresh.shared.schedule()
            } catch {
                let detail = error.localizedDescription
                SharedLog.append("VPN", "启动失败：\(detail)", source: .app)
                vpnSetupError = detail
                showToast("VPN 启动失败，请查看提示")
            }
        }
    }

    func stopTunnel() {
        TunnelManager.shared.stop()
    }

    func toggleTunnel() {
        if vpnState == .connected || vpnState == .connecting {
            stopTunnel()
        } else {
            startTunnel()
        }
    }

    func clearCapture() {
        if APIClient.isPushing {
            showToast("正在推送，请等待本次完成")
            return
        }
        captureStore.clear()
        if vpnState == .connected || vpnState == .connecting {
            TunnelManager.shared.send("clear")
        }
        refreshShared()
        showToast("结果已清除 · 可换号重新进入农场")
    }

    func manualImport(codeText: String, gidsText: String) throws {
        let platform = settings.capturePlatform
        let code = codeText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "" : try UrlPolicy.manualCode(codeText)
        let gids = try UrlPolicy.gids(gidsText, ownGid: "")
        guard !code.isEmpty || !gids.isEmpty else {
            throw SettingsError.message("请至少填写 Code 或好友 GID")
        }
        captureStore.setManual(code: code, gids: gids, platform: platform)
        refreshShared()
        showToast("已手动导入")
    }

    func preparePush(automatic: Bool = false) {
        guard let current = captureStore.current() else {
            if !automatic { showToast("结果已过期，请重新获取") }
            return
        }
        let code = settings.pushCode && !current.code.isEmpty
        let friends = settings.pushFriends && !current.gids.isEmpty
        guard code || friends else {
            if !automatic {
                showToast(current.code.isEmpty && current.gids.isEmpty ? "没有可推送的数据" : "推送内容已在设置中关闭")
            }
            return
        }
        guard !settings.servers.isEmpty else {
            if automatic { showToast("添加服务器后即可推送") } else { selectedTab = 3 }
            return
        }
        Task {
            let groups = await ServerDirectory.load(platform: current.platform, settings: store)
            guard groups.contains(where: { $0.ok }) else {
                showToast(groups.count == 1 ? (groups[0].error ?? "服务器无法连接") : "所有服务器都无法连接")
                return
            }
            let detected = ServerDirectory.detect(groups, capture: current)
            let last = ServerDirectory.previous(groups, capture: current, settings: store)
            pushDestination = PushDestination(
                capture: current,
                groups: groups,
                code: code,
                friends: friends,
                recommended: detected ?? last,
                detected: detected != nil
            )
        }
    }

    func push(choice: ServerDirectory.Choice, destination: PushDestination) async {
        guard let client = choice.group.client, choice.group.current else {
            showToast("服务器配置已变化，请重新选择")
            return
        }
        busy = true
        defer { busy = false }
        do {
            try await client.push(
                targetId: choice.account.id,
                capture: destination.capture,
                code: destination.code,
                friends: destination.friends
            )
            if client.isCurrent {
                store.setTarget(serverId: client.serverId, account: choice.account)
                settings = store.load()
            }
            pushDestination = nil
            showToast("已推送到 \(ServerDirectory.describe(choice, settings: store))")
            refreshShared()
            refreshAccounts()
            if destination.code {
                watchOnline(client: client, account: choice.account)
            }
        } catch {
            showToast(error.localizedDescription)
        }
    }

    func createAccount(group: ServerGroup, baseName: String, destination: PushDestination) async {
        guard let client = group.client, group.current else {
            showToast("服务器配置已变化，请重新选择")
            return
        }
        busy = true
        defer { busy = false }
        do {
            let nonce = String(UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased().prefix(12))
            let actual = try AccountNamePolicy.unique(baseName, nonce: nonce)
            let created = try await client.create(
                baseName: baseName,
                actualName: actual,
                capture: destination.capture,
                friends: destination.friends
            )
            settings = store.load()
            pushDestination = nil
            showToast("已新增账号 \(created.name)")
            refreshShared()
            refreshAccounts()
            watchOnline(client: client, account: created)
        } catch {
            showToast(error.localizedDescription)
        }
    }

    private func watchOnline(client: APIClient, account: Account) {
        Task {
            for _ in 0..<4 {
                try? await Task.sleep(nanoseconds: 4_000_000_000)
                guard client.isCurrent else { return }
                let accounts = (try? await client.accounts()) ?? []
                if let current = accounts.first(where: { $0.id == account.id && $0.uuid == account.uuid }),
                   (try? await client.connected(current)) == true {
                    online[onlineKey(serverId: client.serverId, connectionId: client.connectionId, accountId: account.id)] = true
                    SharedLog.append("服务器", "推送的账号已上线", source: .app)
                    showToast(account.name + " 已上线")
                    return
                }
            }
            SharedLog.append("服务器", "推送已完成，账号尚未上线", source: .app)
        }
    }

    // MARK: - Relogin / notifications

    func requestRelogin(_ group: ServerGroup, _ account: Account) {
        reloginPrompt = ReloginPrompt(
            serverId: group.server.id,
            accountId: account.id,
            accountName: account.name,
            platform: account.platform
        )
    }

    func confirmRelogin(_ prompt: ReloginPrompt) async {
        guard let platform = CapturePlatform(rawValue: prompt.platform) else { return }
        guard vpnState != .connected, vpnState != .connecting else {
            showToast("请先停止当前获取")
            return
        }
        guard let server = settings.server(id: prompt.serverId) else {
            showToast("服务器配置已变化")
            return
        }
        do {
            let client = try APIClient(server: server)
            guard let account = (try? await client.accounts())?.first(where: { $0.id == prompt.accountId }) else {
                showToast("账号已不存在")
                return
            }
            updateSettings { $0.capturePlatform = platform }
            store.setTarget(serverId: server.id, account: account)
            settings = store.load()
            captureStore.clear()
            refreshShared()
            reloginPrompt = nil
            if platform == .wx {
                wxLoginPreset = WxLoginPreset(serverId: server.id, targetId: account.id)
                showWxLogin = true
            } else {
                selectedTab = 0
                showToast("请先在 QQ 中切换到这个账号，然后开始获取")
            }
        } catch {
            showToast(error.localizedDescription)
        }
    }

    func handleNotificationTap(userInfo: [AnyHashable: Any]) {
        guard let serverId = userInfo[AlertNotifier.accountKey] as? String else { return }
        selectedTab = 1
        refreshAccounts()
        guard let accountId = userInfo[AlertNotifier.accountIdKey] as? String else {
            selectedTab = 3
            return
        }
        let expectedUuid = userInfo[AlertNotifier.accountUUIDKey] as? String
        let expectedConnection = userInfo[AlertNotifier.connectionKey] as? String
        Task {
            guard let server = settings.server(id: serverId),
                  expectedConnection == server.connectionId,
                  let client = try? APIClient(server: server) else {
                showToast("服务器配置已变化，这条通知已失效")
                return
            }
            let accounts = (try? await client.accounts()) ?? []
            guard let account = accounts.first(where: { $0.id == accountId }) else {
                showToast("通知对应的账号已不存在")
                return
            }
            if let expectedUuid, !expectedUuid.isEmpty, expectedUuid != account.uuid {
                showToast("原账号已被删除或替换，通知已失效")
                return
            }
            reloginPrompt = ReloginPrompt(
                serverId: serverId, accountId: account.id,
                accountName: account.name, platform: account.platform
            )
        }
    }

    // MARK: - Certificate

    func refreshCertificateTrust() {
        certExists = CertificateAuthority.shared.exists
        guard certExists else {
            certTrusted = nil
            return
        }
        guard !trustCheckRunning else { return }
        trustCheckRunning = true
        Task {
            let trusted = await Task.detached(priority: .userInitiated) {
                CertificateAuthority.shared.isTrustedBySystem()
            }.value
            certTrusted = trusted
            trustCheckRunning = false
        }
    }

    func generateCertificate() async {
        do {
            try await Task.detached(priority: .userInitiated) {
                _ = try CertificateAuthority.shared.generate()
            }.value
            showToast("已生成证书，下一步导出并安装")
            refreshCertificateTrust()
        } catch {
            showToast(error.localizedDescription)
        }
    }

    func deleteCertificate() async {
        guard vpnState != .connected, vpnState != .connecting else {
            showToast("请先停止 VPN 再移除证书")
            return
        }
        do {
            try await Task.detached(priority: .userInitiated) {
                try CertificateAuthority.shared.deleteLocal()
            }.value
            showToast("本地证书与私钥已删除")
            refreshCertificateTrust()
        } catch {
            showToast(error.localizedDescription)
        }
    }

    func exportCertificateURL() -> URL? {
        do {
            let data = try CertificateAuthority.shared.certificateDER()
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("FarmHelper-LocalCA.cer")
            try data.write(to: url, options: .atomic)
            return url
        } catch {
            showToast(error.localizedDescription)
            return nil
        }
    }

    func exportProfileURL() -> URL? {
        do {
            let data = try CertificateAuthority.shared.mobileConfig()
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("FarmHelper-LocalCA.mobileconfig")
            try data.write(to: url, options: .atomic)
            return url
        } catch {
            showToast(error.localizedDescription)
            return nil
        }
    }

    var certificateFingerprint: String {
        (try? CertificateAuthority.shared.fingerprint()) ?? ""
    }

    // MARK: - Updates

    func checkUpdates() async {
        do {
            guard let release = try await UpdateChecker.check(settings: settings) else {
                showToast("已是最新版本")
                return
            }
            updateInfo = release
            showUpdate = true
        } catch {
            showToast(error.localizedDescription)
        }
    }

    // MARK: - Background

    func runBackgroundRefresh() async {
        let status = await TunnelManager.shared.refreshStatus()
        if status == .connected { return }
        await MonitorEngine.checkAllOnce(settings: store)
        BackgroundRefresh.shared.schedule()
    }

    // MARK: - Utilities

    func showToast(_ text: String) {
        toast = text
        lastToast = text
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            guard let self, self.lastToast == text else { return }
            self.toast = nil
        }
    }

    func clearLogs() {
        SharedLog.clear()
        refreshShared()
    }

    func openURL(_ string: String) {
        guard let url = URL(string: string) else { return }
        UIApplication.shared.open(url)
    }

    func openApp(_ scheme: String) {
        guard let url = URL(string: scheme) else { return }
        UIApplication.shared.open(url) { [weak self] success in
            if !success {
                Task { @MainActor in self?.showToast("未安装对应应用") }
            }
        }
    }
}
