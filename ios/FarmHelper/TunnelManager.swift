import Foundation
import NetworkExtension
import Security

final class TunnelManager: @unchecked Sendable {
    static let shared = TunnelManager()
    static let providerBundleIdentifier = "io.github.ccpopy.farmhelper.tunnel"

    private var cached: NETunnelProviderManager?

    private init() {}

    func currentManager() async throws -> NETunnelProviderManager {
        if let cached { return cached }
        let managers = try await Self.loadAll()
        if let existing = managers.first(where: {
            ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == Self.providerBundleIdentifier
        }) {
            cached = existing
            return existing
        }
        let manager = NETunnelProviderManager()
        let proto = NETunnelProviderProtocol()
        proto.providerBundleIdentifier = Self.providerBundleIdentifier
        proto.serverAddress = "127.0.0.1"
        proto.includeAllNetworks = false
        proto.excludeLocalNetworks = true
        manager.protocolConfiguration = proto
        manager.localizedDescription = "农场助手"
        manager.isEnabled = true
        try await Self.save(manager)
        cached = manager
        return manager
    }

    func start(settings: AppSettings) async throws {
        let manager = try await currentManager()
        guard let proto = manager.protocolConfiguration as? NETunnelProviderProtocol else {
            throw TunnelError.invalid("VPN 配置不可用")
        }
        var random = [UInt8](repeating: 0, count: 24)
        guard SecRandomCopyBytes(kSecRandomDefault, random.count, &random) == errSecSuccess else {
            throw TunnelError.invalid("无法生成本地代理凭据")
        }
        let password = Data(random).base64EncodedString()
        proto.providerConfiguration = [
            "platform": settings.capturePlatform.rawValue,
            "passthrough": settings.passthrough,
            "friendCapture": settings.friendCapture,
            "socksPassword": password,
        ]
        proto.serverAddress = "127.0.0.1"
        proto.includeAllNetworks = false
        proto.excludeLocalNetworks = true
        manager.protocolConfiguration = proto
        manager.isEnabled = true
        try await Self.save(manager)
        try await Self.load(manager)
        try manager.connection.startVPNTunnel()
    }

    func stop() {
        if let cached {
            cached.connection.stopVPNTunnel()
            return
        }
        Task {
            let all = (try? await Self.loadAll()) ?? []
            if let manager = all.first(where: {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == Self.providerBundleIdentifier
            }) {
                cached = manager
                manager.connection.stopVPNTunnel()
            }
        }
    }

    var status: NEVPNStatus {
        cached?.connection.status ?? .invalid
    }

    func refreshStatus() async -> NEVPNStatus {
        if cached == nil {
            let managers = (try? await Self.loadAll()) ?? []
            cached = managers.first {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == Self.providerBundleIdentifier
            }
        }
        return cached?.connection.status ?? .invalid
    }

    func send(_ command: String) {
        Task {
            let manager: NETunnelProviderManager?
            if let cached {
                manager = cached
            } else {
                let all = (try? await Self.loadAll()) ?? []
                manager = all.first {
                    ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == Self.providerBundleIdentifier
                }
                cached = manager
            }
            guard let manager,
                  let session = manager.connection as? NETunnelProviderSession else { return }
            try? session.sendProviderMessage(Data(command.utf8)) { _ in }
        }
    }

    private static func loadAll() async throws -> [NETunnelProviderManager] {
        try await withCheckedThrowingContinuation { continuation in
            NETunnelProviderManager.loadAllFromPreferences { managers, error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: managers ?? [])
                }
            }
        }
    }

    private static func save(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.saveToPreferences { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }

    private static func load(_ manager: NETunnelProviderManager) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            manager.loadFromPreferences { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }
    }
}
