import Foundation

/// Pure validation shared by capture, manual input and server requests.
enum UrlPolicy {
    static let farmHost = "gate-obt.nqf.qq.com"
    static let repository = "ccpopy/qq-farm-code-helper"

    static func server(_ input: String, allowHTTP: Bool) throws -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              trimmed.rangeOfCharacter(from: .controlCharacters) == nil,
              var components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              scheme == "https" || (allowHTTP && scheme == "http"),
              let host = components.host, !host.isEmpty,
              components.user == nil, components.password == nil,
              components.query == nil, components.fragment == nil else {
            throw SettingsError.message("请填写 HTTPS 地址，不要附带 Token、查询参数或用户名；HTTP 需在设置中单独允许")
        }
        if let port = components.port, port <= 0 || port > 65535 {
            throw SettingsError.message("服务器端口不正确")
        }
        components.scheme = scheme
        var value = components.string ?? trimmed
        while value.hasSuffix("/") { value.removeLast() }
        guard !value.isEmpty else { throw SettingsError.message("服务器地址格式不正确") }
        return value
    }

    static func captureCode(requestTarget: String, host: String) throws -> String {
        guard host.caseInsensitiveCompare(farmHost) == .orderedSame else {
            throw SettingsError.message("不是目标农场网关")
        }
        guard let components = URLComponents(string: requestTarget) else {
            throw SettingsError.message("登录地址格式不正确")
        }
        if let scheme = components.scheme {
            let lower = scheme.lowercased()
            guard lower == "wss" || lower == "https",
                  components.host?.caseInsensitiveCompare(farmHost) == .orderedSame,
                  components.user == nil, components.password == nil else {
                throw SettingsError.message("登录地址来源不正确")
            }
            if let port = components.port, port != 443 {
                throw SettingsError.message("登录地址来源不正确")
            }
        } else if components.host != nil || components.user != nil || components.password != nil {
            throw SettingsError.message("登录地址来源不正确")
        }
        guard components.percentEncodedPath == "/prod/ws", components.fragment == nil else {
            throw SettingsError.message("不是农场登录路径")
        }

        var found: String?
        if let query = components.percentEncodedQuery {
            for pair in query.split(separator: "&", omittingEmptySubsequences: false) {
                let parts = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
                let key = try decode(String(parts[0]))
                guard key == "code" else { continue }
                guard found == nil, parts.count == 2 else {
                    throw SettingsError.message("Code 参数重复或缺失")
                }
                found = try validateCode(try decode(String(parts[1])))
            }
        }
        guard let code = found else { throw SettingsError.message("未找到 Code") }
        return code
    }

    static func manualCode(_ input: String) throws -> String {
        let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.contains("://") { return try captureCode(requestTarget: value, host: farmHost) }
        return try validateCode(value)
    }

    private static func validateCode(_ value: String) throws -> String {
        guard !value.isEmpty, value.count <= 4096,
              value.rangeOfCharacter(from: .whitespacesAndNewlines) == nil,
              value.rangeOfCharacter(from: .controlCharacters) == nil else {
            throw SettingsError.message("Code 为空、过长或包含空白字符")
        }
        return value
    }

    private static func decode(_ value: String) throws -> String {
        // Query values are percent encoded; a literal '+' in a login Code is not a form space.
        guard let decoded = value.replacingOccurrences(of: "+", with: "%2B").removingPercentEncoding else {
            throw SettingsError.message("参数编码不正确")
        }
        return decoded
    }

    static func gids(_ input: String, ownGid: String) throws -> [String] {
        var seen = Set<String>()
        var result: [String] = []
        let parts = input.components(separatedBy: CharacterSet(charactersIn: ",，;； \t\r\n"))
        for part in parts where !part.isEmpty {
            guard part.range(of: "^[1-9][0-9]{0,15}$", options: .regularExpression) != nil,
                  let number = UInt64(part), number <= 9_007_199_254_740_991 else {
                throw SettingsError.message("GID 必须是正整数")
            }
            let normalized = String(number)
            guard normalized != ownGid, seen.insert(normalized).inserted else { continue }
            result.append(normalized)
        }
        return result
    }

    static func hostWithoutPort(_ value: String) -> String {
        value.hasSuffix(":443") ? String(value.dropLast(4)) : value
    }
}
