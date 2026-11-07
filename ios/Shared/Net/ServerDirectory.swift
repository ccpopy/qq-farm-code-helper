import Foundation

struct ServerGroup: Identifiable, Sendable {
    let server: ServerProfile
    let client: APIClient?
    let accounts: [Account]
    let error: String?
    let connectionId: String

    var id: String { server.id }
    var ok: Bool { error == nil }

    var current: Bool {
        let settings = SharedSettings.shared
        guard let latest = settings.server(id: server.id),
              latest.ready(token: settings.token(for: server.id)) else { return false }
        return latest.connectionId == connectionId
    }
}

/// Reads every saved server at once, so a slow or offline server never hides the others.
enum ServerDirectory {
    struct Choice: Sendable {
        let group: ServerGroup
        let account: Account
    }

    static func load(platform: CapturePlatform?, settings: SharedSettings = .shared) async -> [ServerGroup] {
        let servers = settings.readyServers()
        guard !servers.isEmpty else { return [] }
        return await withTaskGroup(of: (Int, ServerGroup).self) { group in
            for (index, server) in servers.enumerated() {
                group.addTask {
                    (index, await read(server: server, platform: platform, settings: settings))
                }
            }
            var result = [ServerGroup?](repeating: nil, count: servers.count)
            for await (index, value) in group { result[index] = value }
            return result.compactMap { $0 }
        }
    }

    static func read(server: ServerProfile, platform: CapturePlatform?, settings: SharedSettings = .shared) async -> ServerGroup {
        let connection = server.connectionId
        do {
            let client = try APIClient(server: server, settings: settings)
            var accounts = try await client.accounts()
            if let platform {
                accounts.removeAll { $0.platform != platform.rawValue }
            }
            return ServerGroup(server: server, client: client, accounts: accounts, error: nil, connectionId: connection)
        } catch {
            return ServerGroup(server: server, client: nil, accounts: [], error: reason(error), connectionId: connection)
        }
    }

    static func reason(_ error: Error) -> String {
        if let api = error as? APIError {
            if api == .auth { return "Token 无效" }
            if api == .temporary { return "无法连接" }
            let text = api.errorDescription ?? "读取失败"
            return text.count > 60 || text.contains("://") ? "读取失败" : text
        }
        let text = error.localizedDescription
        return text.count > 60 || text.contains("://") ? "读取失败" : text
    }

    /// The account whose game identity matches the capture, only if exactly one exists across servers.
    static func detect(_ groups: [ServerGroup], capture: CaptureSession) -> Choice? {
        guard !capture.ownGid.isEmpty else { return nil }
        var found: Choice?
        for group in groups where group.ok && group.current {
            guard let client = group.client else { continue }
            for account in group.accounts {
                guard account.platform == capture.platform.rawValue,
                      client.knownGid(account) == capture.ownGid else { continue }
                if found != nil { return nil }
                found = Choice(group: group, account: account)
            }
        }
        return found
    }

    /// Where the user pushed last time, unless the captured identity contradicts it.
    static func previous(_ groups: [ServerGroup], capture: CaptureSession, settings: SharedSettings = .shared) -> Choice? {
        let last = settings.load().lastServer
        for group in groups where group.ok && group.current && group.server.id == last {
            guard let client = group.client,
                  let account = group.accounts.first(where: { $0.id == group.server.targetId }) else { continue }
            let known = client.knownGid(account)
            if capture.ownGid.isEmpty || known.isEmpty || capture.ownGid == known {
                return Choice(group: group, account: account)
            }
        }
        return nil
    }

    static func allCurrent(_ groups: [ServerGroup]) -> Bool {
        !groups.contains { $0.ok && !$0.current }
    }

    static func accountCount(_ groups: [ServerGroup]) -> Int {
        groups.reduce(0) { $0 + $1.accounts.count }
    }

    static func describe(_ choice: Choice, settings: SharedSettings = .shared) -> String {
        let many = settings.readyServers().count > 1
        return (many ? choice.group.server.label + " · " : "") + choice.account.name
    }
}
