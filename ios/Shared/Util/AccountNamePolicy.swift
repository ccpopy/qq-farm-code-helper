import Foundation

/// The upstream treats an ID-less request with an existing remark as a relogin.
enum AccountNamePolicy {
    static func base(_ input: String) throws -> String {
        let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, value.count <= 60,
              value.rangeOfCharacter(from: .controlCharacters) == nil else {
            throw SettingsError.message("账号备注须为 1–60 个字符，不能包含换行或控制字符")
        }
        return value
    }

    static func unique(_ input: String, nonce: String) throws -> String {
        guard nonce.range(of: "^[a-f0-9]{12}$", options: .regularExpression) != nil else {
            throw SettingsError.message("新增账号标识无效")
        }
        return try base(input) + " · 手机-" + nonce
    }

    static func validate(_ input: String, _ actual: String) throws {
        let prefix = try base(input) + " · 手机-"
        guard actual.hasPrefix(prefix) else {
            throw SettingsError.message("新增账号备注与确认内容不一致")
        }
        let suffix = String(actual.dropFirst(prefix.count))
        guard suffix.range(of: "^[a-f0-9]{12}$", options: .regularExpression) != nil else {
            throw SettingsError.message("新增账号备注与确认内容不一致")
        }
    }
}
