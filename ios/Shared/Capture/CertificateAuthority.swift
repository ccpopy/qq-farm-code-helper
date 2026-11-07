import CryptoKit
import Foundation
import Network
import Security

enum CertificateError: LocalizedError {
    case missingKey
    case keychain(OSStatus)
    case invalid(String)
    case timeout

    var errorDescription: String? {
        switch self {
        case .missingKey: return "本地证书私钥不存在，请移除旧证书并重新生成"
        case .keychain(let status): return "证书存储失败（\(status)）"
        case .invalid(let text): return text
        case .timeout: return "证书检查超时"
        }
    }
}

/// The local CA is shared by the app and the tunnel extension through the Keychain access group.
/// The certificate itself is also written to the App Group so it can be exported and installed.
final class CertificateAuthority: @unchecked Sendable {
    static let shared = CertificateAuthority()
    static let commonName = "Farm Helper Local CA"

    private let caTag = "farmhelper.local-ca.rsa.v1"
    private let leafPrefix = "farmhelper.leaf."
    private let certificateName = "local-ca.cer"
    private let lock = NSLock()
    private var leafCache: [String: (identity: SecIdentity, certificate: SecCertificate, chain: [SecCertificate])] = [:]

    // MARK: - Paths

    private var certificateURL: URL {
        if let container = AppGroup.containerURL {
            return container.appendingPathComponent(certificateName)
        }
        return URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(certificateName)
    }

    var exists: Bool {
        FileManager.default.fileExists(atPath: certificateURL.path) && copyKey(tag: caTag) != nil
    }

    // MARK: - CA material

    func certificateDER() throws -> Data {
        if let data = try? Data(contentsOf: certificateURL),
           SecCertificateCreateWithData(nil, data as CFData) != nil {
            return data
        }
        return try generate()
    }

    func fingerprint() throws -> String {
        let digest = SHA256.hash(data: try certificateDER())
        return digest.map { String(format: "%02X", $0) }.joined(separator: ":")
    }

    /// A configuration profile is the supported way to install a root CA on iOS. The profile
    /// only contains the public certificate; the private key stays in the Keychain.
    func mobileConfig() throws -> Data {
        let der = try certificateDER()
        let payload: [String: Any] = [
            "PayloadCertificateFileName": "FarmHelper-LocalCA.cer",
            "PayloadContent": der,
            "PayloadDescription": "农场助手本地抓包 CA",
            "PayloadDisplayName": Self.commonName,
            "PayloadIdentifier": "io.github.ccpopy.farmhelper.ca",
            "PayloadType": "com.apple.security.root",
            "PayloadUUID": UUID().uuidString,
            "PayloadVersion": 1,
        ]
        let profile: [String: Any] = [
            "PayloadContent": [payload],
            "PayloadDisplayName": "农场助手本地 CA",
            "PayloadDescription": "安装后请在证书信任设置中打开完全信任",
            "PayloadIdentifier": "io.github.ccpopy.farmhelper.profile",
            "PayloadOrganization": "Farm Helper",
            "PayloadRemovalDisallowed": false,
            "PayloadType": "Configuration",
            "PayloadUUID": UUID().uuidString,
            "PayloadVersion": 1,
        ]
        return try PropertyListSerialization.data(fromPropertyList: profile, format: .xml, options: 0)
    }

    @discardableResult
    func generate() throws -> Data {
        lock.lock()
        defer { lock.unlock() }
        return try generateUnlocked()
    }

    private func generateUnlocked() throws -> Data {
        let key: SecKey
        let createdKey: Bool
        if let existing = copyKey(tag: caTag) {
            key = existing
            createdKey = false
        } else {
            key = try createPermanentKey(tag: caTag)
            createdKey = true
        }
        if !createdKey, let data = try? Data(contentsOf: certificateURL),
           SecCertificateCreateWithData(nil, data as CFData) != nil {
            return data
        }
        guard let publicKey = SecKeyCopyPublicKey(key) else { throw CertificateError.missingKey }
        let now = Date()
        let data = try makeCertificate(
            subject: name(Self.commonName),
            publicKey: publicKey,
            issuer: name(Self.commonName),
            issuerKey: key,
            isCA: true,
            notBefore: now.addingTimeInterval(-300),
            notAfter: now.addingTimeInterval(365 * 24 * 60 * 60),
            dnsNames: []
        )
        try data.write(to: certificateURL, options: .atomic)
        SharedLog.append("证书", "已生成本地 CA；私钥保留在 Keychain，尚未安装到系统", source: .app)
        return data
    }

    func deleteLocal() throws {
        lock.lock()
        defer { lock.unlock() }
        var query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: Data(caTag.utf8),
        ]
        if let group = AppGroup.keychainAccessGroup { query[kSecAttrAccessGroup as String] = group }
        SecItemDelete(query as CFDictionary)
        try? FileManager.default.removeItem(at: certificateURL)
        leafCache.removeAll()
        deleteLeafMaterialUnlocked()
        SharedLog.append("证书", "助手本地证书与私钥已删除，不可恢复；系统副本需手动移除", source: .app)
    }

    // MARK: - Leaf certificates

    /// `remoteDER` must already be authenticated by the upstream TLS handshake. Its subject,
    /// DNS SANs and validity are copied; the issuer and public key are never copied.
    func leafIdentity(remoteDER: Data?, host: String) throws -> (identity: SecIdentity, certificates: [SecCertificate]) {
        lock.lock()
        defer { lock.unlock() }

        let cacheKey = (remoteDER.map { SHA256.hash(data: $0).map { String(format: "%02x", $0) }.joined() } ?? "none") + "/" + host
        if let cached = leafCache[cacheKey] {
            return (cached.identity, cached.chain)
        }

        let caDER = try generateUnlocked()
        guard let caCertificate = SecCertificateCreateWithData(nil, caDER as CFData),
              let caKey = copyKey(tag: caTag) else { throw CertificateError.missingKey }
        let caInfo = try X509Parser.info(caDER)

        let now = Date()
        var subject: Data
        var dnsNames: [String]
        var notBefore = now.addingTimeInterval(-300)
        var notAfter = now.addingTimeInterval(7 * 24 * 60 * 60)
        if let remoteDER {
            let info = try X509Parser.info(remoteDER)
            subject = info.subject
            dnsNames = info.dnsNames.isEmpty ? [host] : info.dnsNames
            notBefore = max(caInfo.notBefore, info.notBefore)
            notAfter = min(caInfo.notAfter, info.notAfter)
            guard notAfter > notBefore else {
                throw CertificateError.invalid("上游证书与本地 CA 的有效期没有交集")
            }
        } else {
            subject = name(host)
            dnsNames = [host]
        }

        let tag = leafPrefix + UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
        let leafKey = try createPermanentKey(tag: tag)
        guard let publicKey = SecKeyCopyPublicKey(leafKey) else { throw CertificateError.missingKey }
        let leafDER = try makeCertificate(
            subject: subject,
            publicKey: publicKey,
            issuer: caInfo.subject,
            issuerKey: caKey,
            isCA: false,
            notBefore: notBefore,
            notAfter: notAfter,
            dnsNames: dnsNames
        )
        guard let leafCertificate = SecCertificateCreateWithData(nil, leafDER as CFData) else {
            throw CertificateError.invalid("生成的叶子证书无效")
        }
        addCertificate(leafCertificate, label: tag)
        // iOS has no public SecIdentityCreateWithCertificate. The supported route is to put the
        // certificate and private key in the Keychain under the same label, then fetch the
        // virtual identity item (the same approach Signal uses for its device-transfer identity).
        guard let identity = copyIdentity(label: tag) else {
            deleteLeaf(tag: tag)
            throw CertificateError.invalid("无法从 Keychain 组合 TLS 身份")
        }
        let chain = [leafCertificate, caCertificate]
        leafCache[cacheKey] = (identity, leafCertificate, chain)
        return (identity, chain)
    }

    func cleanupLeafMaterial() {
        lock.lock()
        leafCache.removeAll()
        deleteLeafMaterialUnlocked()
        lock.unlock()
    }

    /// Real trust check: run a local TLS server with our leaf and connect to it with system
    /// verification and the farm's hostname. This only reports what iOS actually trusts.
    func isTrustedBySystem(host: String = UrlPolicy.farmHost) -> Bool {
        do {
            let material = try leafIdentity(remoteDER: nil, host: host)
            let listenerParams = try serverParameters(identity: material.identity, chain: material.certificates)
            let listener = try NWListener(using: listenerParams)
            var accepted: NWConnection?
            listener.newConnectionHandler = { connection in
                if accepted == nil { accepted = connection }
                connection.start(queue: .global(qos: .userInitiated))
            }
            guard waitForListener(listener, timeout: 3), let port = listener.port else {
                listener.cancel()
                return false
            }
            let tls = NWProtocolTLS.Options()
            sec_protocol_options_set_tls_server_name(tls.securityProtocolOptions, host)
            sec_protocol_options_add_tls_application_protocol(tls.securityProtocolOptions, "http/1.1")
            let client = NWConnection(
                host: "127.0.0.1", port: port,
                using: NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
            )
            let ready = waitForConnection(client, timeout: 5)
            client.cancel()
            accepted?.cancel()
            listener.cancel()
            return ready
        } catch {
            return false
        }
    }

    // MARK: - Keychain

    private func createPermanentKey(tag: String) throws -> SecKey {
        // Generate outside the Keychain, then import the key with an explicit label. This is the
        // same sequence iOS itself uses for an identity: a labelled private key plus a labelled
        // certificate compose a virtual kSecClassIdentity item.
        let attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeRSA,
            kSecAttrKeySizeInBits as String: 2048,
        ]
        var error: Unmanaged<CFError>?
        guard let generated = SecKeyCreateRandomKey(attributes as CFDictionary, &error) else {
            throw CertificateError.invalid(error?.takeRetainedValue().localizedDescription ?? "无法生成 RSA 密钥")
        }
        var add: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrKeyClass as String: kSecAttrKeyClassPrivate,
            kSecValueRef as String: generated,
            kSecAttrLabel as String: tag,
            kSecAttrApplicationTag as String: Data(tag.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        if let group = AppGroup.keychainAccessGroup {
            add[kSecAttrAccessGroup as String] = group
        }
        let status = SecItemAdd(add as CFDictionary, nil)
        guard status == errSecSuccess || status == errSecDuplicateItem else {
            throw CertificateError.keychain(status)
        }
        return copyKey(tag: tag) ?? generated
    }

    private func copyKey(tag: String) -> SecKey? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: Data(tag.utf8),
            kSecReturnRef as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        if let group = AppGroup.keychainAccessGroup { query[kSecAttrAccessGroup as String] = group }
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else { return nil }
        return item as? SecKey
    }

    private func copyIdentity(label: String) -> SecIdentity? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassIdentity,
            kSecAttrLabel as String: label,
            kSecReturnRef as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        if let group = AppGroup.keychainAccessGroup { query[kSecAttrAccessGroup as String] = group }
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess, let item else { return nil }
        return (item as! SecIdentity)
    }

    private func addCertificate(_ certificate: SecCertificate, label: String) {
        var attributes: [String: Any] = [
            kSecClass as String: kSecClassCertificate,
            kSecValueRef as String: certificate,
            kSecAttrLabel as String: label,
        ]
        if let group = AppGroup.keychainAccessGroup {
            attributes[kSecAttrAccessGroup as String] = group
        }
        SecItemAdd(attributes as CFDictionary, nil)
    }

    private func deleteLeaf(tag: String) {
        var identityQuery: [String: Any] = [
            kSecClass as String: kSecClassIdentity,
            kSecAttrLabel as String: tag,
        ]
        if let group = AppGroup.keychainAccessGroup { identityQuery[kSecAttrAccessGroup as String] = group }
        SecItemDelete(identityQuery as CFDictionary)
        var query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: Data(tag.utf8),
        ]
        if let group = AppGroup.keychainAccessGroup { query[kSecAttrAccessGroup as String] = group }
        SecItemDelete(query as CFDictionary)
        var certQuery: [String: Any] = [
            kSecClass as String: kSecClassCertificate,
            kSecAttrLabel as String: tag,
        ]
        if let group = AppGroup.keychainAccessGroup { certQuery[kSecAttrAccessGroup as String] = group }
        SecItemDelete(certQuery as CFDictionary)
    }

    private func deleteLeafMaterialUnlocked() {
        var keyQuery: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrKeyClass as String: kSecAttrKeyClassPrivate,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        if let group = AppGroup.keychainAccessGroup { keyQuery[kSecAttrAccessGroup as String] = group }
        var keyItems: CFTypeRef?
        if SecItemCopyMatching(keyQuery as CFDictionary, &keyItems) == errSecSuccess,
           let items = keyItems as? [[String: Any]] {
            for item in items {
                guard let tag = item[kSecAttrApplicationTag as String] as? Data,
                      let text = String(data: tag, encoding: .utf8), text.hasPrefix(leafPrefix) else { continue }
                deleteLeaf(tag: text)
            }
        }
        var certQuery: [String: Any] = [
            kSecClass as String: kSecClassCertificate,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        if let group = AppGroup.keychainAccessGroup { certQuery[kSecAttrAccessGroup as String] = group }
        var certItems: CFTypeRef?
        if SecItemCopyMatching(certQuery as CFDictionary, &certItems) == errSecSuccess,
           let items = certItems as? [[String: Any]] {
            for item in items {
                guard let label = item[kSecAttrLabel as String] as? String, label.hasPrefix(leafPrefix) else { continue }
                deleteLeaf(tag: label)
            }
        }

        var identityQuery: [String: Any] = [
            kSecClass as String: kSecClassIdentity,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        if let group = AppGroup.keychainAccessGroup { identityQuery[kSecAttrAccessGroup as String] = group }
        var identityItems: CFTypeRef?
        if SecItemCopyMatching(identityQuery as CFDictionary, &identityItems) == errSecSuccess,
           let items = identityItems as? [[String: Any]] {
            for item in items {
                guard let label = item[kSecAttrLabel as String] as? String, label.hasPrefix(leafPrefix) else { continue }
                var deleteQuery: [String: Any] = [
                    kSecClass as String: kSecClassIdentity,
                    kSecAttrLabel as String: label,
                ]
                if let group = AppGroup.keychainAccessGroup { deleteQuery[kSecAttrAccessGroup as String] = group }
                SecItemDelete(deleteQuery as CFDictionary)
            }
        }
    }

    // MARK: - DER certificate construction

    private func name(_ value: String) -> Data {
        DER.sequence(DER.set(DER.sequence(DER.oid("2.5.4.3"), DER.utf8(value))))
    }

    private func extensionField(_ oid: String, critical: Bool, content: Data) -> Data {
        if critical {
            return DER.sequence(DER.oid(oid), DER.boolean(true), DER.tlv(0x04, content))
        }
        return DER.sequence(DER.oid(oid), DER.tlv(0x04, content))
    }

    private func subjectPublicKeyInfo(_ key: SecKey) throws -> Data {
        var error: Unmanaged<CFError>?
        guard let representation = SecKeyCopyExternalRepresentation(key, &error) else {
            throw CertificateError.invalid(error?.takeRetainedValue().localizedDescription ?? "无法读取公钥")
        }
        let data = representation as Data
        return DER.sequence(
            DER.sequence(DER.oid("1.2.840.113549.1.1.1"), DER.null()),
            DER.bitString(data, unused: 0)
        )
    }

    private func makeCertificate(
        subject: Data,
        publicKey: SecKey,
        issuer: Data,
        issuerKey: SecKey,
        isCA: Bool,
        notBefore: Date,
        notAfter: Date,
        dnsNames: [String]
    ) throws -> Data {
        guard notAfter > notBefore else { throw CertificateError.invalid("证书有效期没有交集") }
        let signatureAlgorithm = DER.sequence(DER.oid("1.2.840.113549.1.1.11"), DER.null())
        let spki = try subjectPublicKeyInfo(publicKey)

        let extensions: Data
        if isCA {
            extensions = DER.sequence(
                extensionField("2.5.29.19", critical: true,
                               content: DER.sequence(DER.boolean(true), DER.integer(0))),
                extensionField("2.5.29.15", critical: true,
                               content: DER.bitString(Data([0x06]), unused: 1))
            )
        } else {
            let names = dnsNames.map { DER.contextPrimitive(2, Data($0.utf8)) }
            extensions = DER.sequence(
                extensionField("2.5.29.19", critical: true, content: DER.sequence()),
                extensionField("2.5.29.15", critical: true,
                               content: DER.bitString(Data([0xa0]), unused: 5)),
                extensionField("2.5.29.37", critical: false,
                               content: DER.sequence(DER.oid("1.3.6.1.5.5.7.3.1"))),
                extensionField("2.5.29.17", critical: false, content: DER.sequence(names))
            )
        }

        var serial = [UInt8](repeating: 0, count: 16)
        _ = SecRandomCopyBytes(kSecRandomDefault, serial.count, &serial)
        serial[0] |= 0x80

        let tbs = DER.sequence(
            DER.explicit(0xa0, DER.integer(2)),
            DER.integer(Data(serial)),
            signatureAlgorithm,
            issuer,
            DER.sequence(DER.utcTime(notBefore), DER.utcTime(notAfter)),
            subject,
            spki,
            DER.explicit(0xa3, extensions)
        )
        var error: Unmanaged<CFError>?
        guard let signatureCF = SecKeyCreateSignature(
            issuerKey, .rsaSignatureMessagePKCS1v15SHA256, tbs as CFData, &error
        ) else {
            throw CertificateError.invalid(error?.takeRetainedValue().localizedDescription ?? "证书签名失败")
        }
        let certificate = DER.sequence(tbs, signatureAlgorithm, DER.bitString(signatureCF as Data, unused: 0))
        guard SecCertificateCreateWithData(nil, certificate as CFData) != nil else {
            throw CertificateError.invalid("生成的证书编码无效")
        }
        return certificate
    }

    // MARK: - Network helpers

    private func serverParameters(identity: SecIdentity, chain: [SecCertificate]) throws -> NWParameters {
        let tls = NWProtocolTLS.Options()
        let secIdentity = sec_identity_create_with_certificates(identity, (chain as NSArray) as CFArray)
            ?? sec_identity_create(identity)
        guard let secIdentity else { throw CertificateError.invalid("无法创建 TLS 身份") }
        sec_protocol_options_set_local_identity(tls.securityProtocolOptions, secIdentity)
        sec_protocol_options_add_tls_application_protocol(tls.securityProtocolOptions, "http/1.1")
        let parameters = NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
        parameters.allowLocalEndpointReuse = true
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        return parameters
    }

    private func waitForListener(_ listener: NWListener, timeout: TimeInterval) -> Bool {
        if listener.state == .ready { return true }
        return waitUntil(timeout: timeout) { fulfill in
            listener.stateUpdateHandler = { state in
                switch state {
                case .ready: fulfill(true)
                case .failed, .cancelled: fulfill(false)
                default: break
                }
            }
            if listener.state == .setup {
                listener.start(queue: .global(qos: .userInitiated))
            }
        }
    }

    private func waitForConnection(_ connection: NWConnection, timeout: TimeInterval) -> Bool {
        if connection.state == .ready { return true }
        return waitUntil(timeout: timeout) { fulfill in
            connection.stateUpdateHandler = { state in
                switch state {
                case .ready: fulfill(true)
                case .failed, .cancelled, .waiting: fulfill(false)
                default: break
                }
            }
            if connection.state == .setup {
                connection.start(queue: .global(qos: .userInitiated))
            }
        }
    }

    private func waitUntil(timeout: TimeInterval, _ install: (@escaping (Bool) -> Void) -> Void) -> Bool {
        let condition = NSCondition()
        var finished = false
        var value = false
        install { result in
            condition.lock()
            if !finished {
                finished = true
                value = result
                condition.broadcast()
            }
            condition.unlock()
        }
        let deadline = Date().addingTimeInterval(timeout)
        condition.lock()
        while !finished, Date() < deadline {
            condition.wait(until: Date().addingTimeInterval(0.1))
        }
        let result = finished ? value : false
        condition.unlock()
        return result
    }
}
