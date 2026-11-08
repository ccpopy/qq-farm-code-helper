import Foundation
import Security

enum KeychainStore {
    private static let tokenService = "io.github.ccpopy.farmhelper.token"
    private static let caService = "io.github.ccpopy.farmhelper.ca"
    private static let captureService = "io.github.ccpopy.farmhelper.capture"

    private static func accessGroup() -> String? {
        guard let value = AppGroup.keychainAccessGroup, !value.isEmpty else { return nil }
        return value
    }

    static func token(serverId: String) -> String? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: tokenService,
            kSecAttrAccount as String: serverId,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        if let group = accessGroup() { query[kSecAttrAccessGroup as String] = group }
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let value = String(data: data, encoding: .utf8) else { return nil }
        return value
    }

    static func setToken(_ token: String, serverId: String) throws {
        let data = Data(token.utf8)
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: tokenService,
            kSecAttrAccount as String: serverId,
        ]
        if let group = accessGroup() { query[kSecAttrAccessGroup as String] = group }
        let updateAttributes: [String: Any] = [kSecValueData as String: data]
        let status = SecItemUpdate(query as CFDictionary, updateAttributes as CFDictionary)
        if status == errSecItemNotFound {
            var insert = query
            insert[kSecValueData as String] = data
            insert[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let added = SecItemAdd(insert as CFDictionary, nil)
            guard added == errSecSuccess else { throw SettingsError.keychain(added) }
        } else if status != errSecSuccess {
            throw SettingsError.keychain(status)
        }
    }

    static func deleteToken(serverId: String) {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: tokenService,
            kSecAttrAccount as String: serverId,
        ]
        if let group = accessGroup() { query[kSecAttrAccessGroup as String] = group }
        SecItemDelete(query as CFDictionary)
    }

    // MARK: - Capture encryption key

    /// The capture snapshot lives in the App Group so both processes can see it. Its Code is
    /// encrypted at rest with this device-only Keychain key.
    static func captureKeyData() -> Data? {
        let account = "capture.aes.v1"
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: captureService,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        if let group = accessGroup() { query[kSecAttrAccessGroup as String] = group }
        var item: CFTypeRef?
        if SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
           let data = item as? Data {
            return data
        }
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { return nil }
        let data = Data(bytes)
        var attributes: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: captureService,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        if let group = accessGroup() { attributes[kSecAttrAccessGroup as String] = group }
        let status = SecItemAdd(attributes as CFDictionary, nil)
        if status == errSecDuplicateItem {
            var retry: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: captureService,
                kSecAttrAccount as String: account,
                kSecReturnData as String: true,
                kSecMatchLimit as String: kSecMatchLimitOne,
            ]
            if let group = accessGroup() { retry[kSecAttrAccessGroup as String] = group }
            var existing: CFTypeRef?
            if SecItemCopyMatching(retry as CFDictionary, &existing) == errSecSuccess {
                return existing as? Data
            }
            return nil
        }
        guard status == errSecSuccess else { return nil }
        return data
    }

    // MARK: - Local CA private key

    static func caKeyData() -> Data? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: caService,
            kSecAttrAccount as String: "local-ca.rsa.v1",
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        if let group = accessGroup() { query[kSecAttrAccessGroup as String] = group }
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else { return nil }
        return item as? Data
    }

    static func setCAKeyData(_ data: Data) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: caService,
            kSecAttrAccount as String: "local-ca.rsa.v1",
        ]
        var attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        if let group = accessGroup() {
            attributes[kSecAttrAccessGroup as String] = group
        }
        var insert = query.merging(attributes) { _, new in new }
        if let group = accessGroup() { insert[kSecAttrAccessGroup as String] = group }
        let status = SecItemAdd(insert as CFDictionary, nil)
        if status == errSecDuplicateItem {
            SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        } else if status != errSecSuccess {
            throw SettingsError.keychain(status)
        }
    }

    static func deleteCAKeyData() {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: caService,
            kSecAttrAccount as String: "local-ca.rsa.v1",
        ]
        if let group = accessGroup() { query[kSecAttrAccessGroup as String] = group }
        SecItemDelete(query as CFDictionary)
    }
}
