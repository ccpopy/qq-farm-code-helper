import Foundation

/// QQ numbers and game GIDs are separate namespaces, even when their digits happen to match.
enum AccountIdentity {
    static func qq(_ value: String?) -> String {
        let number = (value ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return number.range(of: "^[1-9][0-9]{4,11}$", options: .regularExpression) != nil ? number : ""
    }

    static func gid(_ value: String?) -> String {
        let number = (value ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard number.range(of: "^[1-9][0-9]{0,15}$", options: .regularExpression) != nil,
              let parsed = UInt64(number), parsed <= 9_007_199_254_740_991 else { return "" }
        return String(parsed)
    }
}

struct Account: Identifiable, Equatable, Sendable {
    let id: String
    let name: String
    let platform: String
    let gid: String
    let uuid: String
    let qq: String
    let running: Bool

    init?(json: [String: Any]) {
        guard let id = json["id"] as? String, !id.isEmpty else { return nil }
        self.id = id
        self.name = (json["name"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? id
        let platform = (json["platform"] as? String) ?? "qq"
        self.platform = platform
        self.gid = AccountIdentity.gid(json["gid"] as? String)
        // The panel uses uin; older servers may only expose qq. Neither is a game GID.
        let uin = AccountIdentity.qq(json["uin"] as? String)
        self.qq = platform == "qq" ? (uin.isEmpty ? AccountIdentity.qq(json["qq"] as? String) : uin) : ""
        if let uuid = json["uuid"] as? String {
            self.uuid = uuid.trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            self.uuid = ""
        }
        self.running = (json["running"] as? Bool) ?? ((json["running"] as? NSNumber)?.boolValue ?? false)
    }

    var identityLabel: String {
        let source = platform == "qq" ? "QQ" : platform == "wx" ? "微信" : platform
        if !qq.isEmpty { return "QQ " + qq }
        if !gid.isEmpty { return source + " · 农场 GID " + gid }
        return source + " · 账号 ID " + id
    }

    var selectionLabel: String { name + "（" + identityLabel + "）" }
    var label: String { selectionLabel + " · " + (running ? "运行中" : "已停止") }
}
