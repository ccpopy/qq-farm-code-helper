import Foundation

enum SettingsError: LocalizedError {
    case message(String)
    case keychain(OSStatus)

    var errorDescription: String? {
        switch self {
        case .message(let text): return text
        case .keychain: return "凭据存储失败，请重新填写 Token"
        }
    }
}

/// Settings live in the App Group so the tunnel extension can read capture options and the
/// monitor account selection. Passwords/tokens stay in the Keychain.
final class SharedSettings: @unchecked Sendable {
    static let shared = SharedSettings()

    private let defaults = AppGroup.defaults
    private let settingsKey = "settings.v1"
    private let lock = NSLock()
    private let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.sortedKeys]
        return encoder
    }()
    private let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()

    // MARK: - Read / write

    func load() -> AppSettings {
        lock.lock()
        defer { lock.unlock() }
        return loadUnlocked()
    }

    private func loadUnlocked() -> AppSettings {
        guard let data = defaults.data(forKey: settingsKey),
              let value = try? decoder.decode(AppSettings.self, from: data) else {
            return AppSettings()
        }
        return value
    }

    func save(_ value: AppSettings) {
        lock.lock()
        saveUnlocked(value)
        lock.unlock()
    }

    private func saveUnlocked(_ value: AppSettings) {
        if let data = try? encoder.encode(value) {
            defaults.set(data, forKey: settingsKey)
        }
    }

    @discardableResult
    func mutate(_ body: (inout AppSettings) -> Void) -> AppSettings {
        lock.lock()
        var value = loadUnlocked()
        body(&value)
        saveUnlocked(value)
        lock.unlock()
        return value
    }

    // MARK: - Servers

    func hasServer() -> Bool { !load().servers.isEmpty }
    func server(id: String?) -> ServerProfile? { load().server(id: id) }

    func token(for serverId: String) -> String {
        KeychainStore.token(serverId: serverId) ?? ""
    }

    func readyServers() -> [ServerProfile] {
        let value = load()
        return value.servers.filter { $0.ready(token: KeychainStore.token(serverId: $0.id) ?? "") }
    }

    func saveServer(existingId: String?, name: String, address: String, token: String, allowHTTP: Bool) throws -> ServerProfile {
        let base = try UrlPolicy.server(address, allowHTTP: allowHTTP)
        let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard label.count <= 20 else { throw SettingsError.message("名称最多 20 个字") }
        let secret = token.trimmingCharacters(in: .whitespacesAndNewlines)

        var value = load()
        let existing = existingId.flatMap { value.server(id: $0) }
        let previousToken = existing.map { KeychainStore.token(serverId: $0.id) ?? "" } ?? ""

        var effectiveToken = secret
        if effectiveToken.isEmpty {
            if existing == nil || base != existing?.baseURL {
                throw SettingsError.message("首次配置或更换服务器地址时，请重新填写管理 Token")
            }
            effectiveToken = previousToken
        }
        guard !effectiveToken.isEmpty, effectiveToken.count <= 8192,
              !effectiveToken.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw SettingsError.message("请填写有效的管理 Token")
        }

        for other in value.servers where other.id != existing?.id && other.baseURL == base {
            throw SettingsError.message("这个服务器已添加过（\(other.label)）")
        }

        var profile = existing ?? ServerProfile(id: value.servers.isEmpty ? "0" : Self.randomId())
        let changed = base != profile.baseURL || effectiveToken != previousToken || profile.connectionId.isEmpty
        profile.baseURL = base
        profile.allowHTTP = allowHTTP
        profile.name = label
        if changed {
            profile.connectionId = UUID().uuidString
            profile.targetId = ""
            profile.targetName = "未选择账号"
            profile.targetGid = ""
            profile.targetPlatform = "qq"
            profile.monitoredIds = []
            profile.identities = [:]
        }
        try KeychainStore.setToken(effectiveToken, serverId: profile.id)
        value.replace(profile)
        save(value)
        return profile
    }

    private static func randomId() -> String {
        String(UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased().prefix(10))
    }

    func removeServer(id: String) {
        var value = load()
        value.removeServer(id: id)
        save(value)
        KeychainStore.deleteToken(serverId: id)
    }

    func setTarget(serverId: String, account: Account) {
        _ = mutate { value in
            guard let index = value.servers.firstIndex(where: { $0.id == serverId }) else { return }
            value.servers[index].targetId = account.id
            value.servers[index].targetName = account.name
            value.servers[index].targetGid = account.gid
            value.servers[index].targetPlatform = account.platform
            value.lastServer = serverId
        }
    }

    func rememberIdentity(serverId: String, connectionId: String, account: Account, capture: CaptureSession) {
        guard account.platform == capture.platform.rawValue else { return }
        _ = mutate { value in
            guard let index = value.servers.firstIndex(where: { $0.id == serverId }),
                  value.servers[index].connectionId == connectionId else { return }
            // A Code-only replacement must invalidate a previous association, not silently reuse it.
            value.servers[index].identities.removeValue(forKey: account.id)
            let gid = AccountIdentity.gid(capture.ownGid)
            guard !account.uuid.isEmpty, !gid.isEmpty else { return }
            value.servers[index].identities[account.id] = RememberedIdentity(
                uuid: account.uuid, platform: account.platform, qq: account.qq, gid: gid
            )
        }
    }

    func rememberedGid(serverId: String, connectionId: String, account: Account) -> String {
        let value = load()
        guard let server = value.server(id: serverId), server.connectionId == connectionId,
              !account.uuid.isEmpty,
              let entry = server.identities[account.id],
              entry.uuid == account.uuid,
              entry.platform == account.platform,
              entry.qq == account.qq else { return "" }
        return AccountIdentity.gid(entry.gid)
    }

    func monitor(serverId: String, ids: Set<String>) {
        _ = mutate { value in
            guard let index = value.servers.firstIndex(where: { $0.id == serverId }) else { return }
            value.servers[index].monitoredIds = ids.sorted()
        }
    }

    func anyMonitored() -> Bool {
        readyServers().contains { !$0.monitoredIds.isEmpty }
    }

    func monitoredCount() -> Int {
        readyServers().reduce(0) { $0 + $1.monitoredIds.count }
    }
}
