import Foundation

struct RememberedIdentity: Codable, Equatable, Sendable {
    var uuid: String
    var platform: String
    var qq: String
    var gid: String
}

/// One saved farm server. The token lives in the Keychain, never in this structure.
struct ServerProfile: Codable, Identifiable, Equatable, Sendable {
    var id: String
    var name: String
    var baseURL: String
    var allowHTTP: Bool
    var connectionId: String
    var targetId: String
    var targetName: String
    var targetGid: String
    var targetPlatform: String
    var monitoredIds: [String]
    var identities: [String: RememberedIdentity]

    init(id: String) {
        self.id = id
        self.name = ""
        self.baseURL = ""
        self.allowHTTP = false
        self.connectionId = ""
        self.targetId = ""
        self.targetName = "未选择账号"
        self.targetGid = ""
        self.targetPlatform = "qq"
        self.monitoredIds = []
        self.identities = [:]
    }

    var label: String { name.isEmpty ? host : name }

    var host: String {
        guard let components = URLComponents(string: baseURL), let host = components.host else { return baseURL }
        if let port = components.port { return "\(host):\(port)" }
        return host
    }

    func ready(token: String) -> Bool { !baseURL.isEmpty && !token.isEmpty }
}

struct AppSettings: Codable, Equatable, Sendable {
    var servers: [ServerProfile] = []
    var capturePlatform: CapturePlatform = .qq
    var friendCaptureQQ: Bool = true
    var friendCaptureWX: Bool = false
    var passthrough: Bool = false
    var autoPush: Bool = true
    var pushCode: Bool = true
    var pushFriends: Bool = true
    var monitorEnabled: Bool = false
    var pollSeconds: Int = 60
    var accelerated: Bool = true
    var mirror: String = "https://ghfast.top"
    var lastServer: String = "0"

    var friendCapture: Bool {
        get { capturePlatform == .qq ? friendCaptureQQ : friendCaptureWX }
        set {
            if capturePlatform == .qq { friendCaptureQQ = newValue } else { friendCaptureWX = newValue }
        }
    }

    func server(id: String?) -> ServerProfile? {
        guard let id else { return nil }
        return servers.first { $0.id == id }
    }

    mutating func replace(_ server: ServerProfile) {
        if let index = servers.firstIndex(where: { $0.id == server.id }) {
            servers[index] = server
        } else {
            servers.append(server)
        }
    }

    mutating func removeServer(id: String) {
        servers.removeAll { $0.id == id }
        if lastServer == id { lastServer = servers.first?.id ?? "0" }
    }
}
