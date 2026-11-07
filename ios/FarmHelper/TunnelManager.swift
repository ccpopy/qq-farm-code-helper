import Darwin
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
        try Self.validateBundle()

        let manager: NETunnelProviderManager
        do {
            manager = try await currentManager()
        } catch {
            throw Self.mapStartError(error)
        }
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
        do {
            try await Self.save(manager)
            try await Self.load(manager)
            try manager.connection.startVPNTunnel()
        } catch {
            throw Self.mapStartError(error)
        }
    }

    /// Fail early with an actionable message when the installed bundle is incomplete.
    private static func validateBundle() throws {
        let plugins = Bundle.main.bundleURL.appendingPathComponent("PlugIns", isDirectory: true)
        guard FileManager.default.fileExists(atPath: plugins.path) else {
            throw TunnelError.invalid("安装包没有 PlugIns 目录，FarmHelperTunnel.appex 未嵌入；请安装完整的签名 App。")
        }
        let appex = plugins.appendingPathComponent("FarmHelperTunnel.appex")
        guard FileManager.default.fileExists(atPath: appex.path) else {
            throw TunnelError.invalid("安装包缺少 FarmHelperTunnel.appex；请重新安装完整应用，不要只复制主 App。")
        }
        let appProfile = Bundle.main.bundleURL.appendingPathComponent("embedded.mobileprovision")
        let tunnelProfile = appex.appendingPathComponent("embedded.mobileprovision")
        let hasAppProfile = FileManager.default.fileExists(atPath: appProfile.path)
        let hasTunnelProfile = FileManager.default.fileExists(atPath: tunnelProfile.path)
        if !hasAppProfile || !hasTunnelProfile {
            SharedLog.append(
                "VPN",
                "签名诊断：未找到 App 或扩展的 embedded.mobileprovision；未签名/重签不完整的安装包会被 Network Extension 拒绝",
                source: .app
            )
        }
    }

    /// Translate the cryptic EPERM from NetworkExtension into the actual signing requirement.
    private static func mapStartError(_ error: Error) -> Error {
        if let tunnel = error as? TunnelError { return tunnel }
        let ns = error as NSError
        let text = ns.localizedDescription
        let lower = text.lowercased()
        let permissionDenied = lower.contains("permission denied")
            || lower.contains("not permitted")
            || (ns.domain == NSPOSIXErrorDomain && ns.code == Int(EPERM))

        if permissionDenied {
            return TunnelError.invalid("""
            系统拒绝创建或启动 VPN（permission denied）。这是安装包的签名/描述文件权限问题，不是服务器或抓包证书问题。
            1. 必须使用付费 Apple Developer 账号；免费个人团队不能使用 Network Extension。
            2. 主 App（io.github.ccpopy.farmhelper）和扩展（io.github.ccpopy.farmhelper.tunnel）的 Provisioning Profile 都必须启用 Network Extensions（packet-tunnel-provider）。
            3. 两个描述文件都必须包含 App Group group.io.github.ccpopy.farmhelper 和共享 Keychain 组 io.github.ccpopy.farmhelper。
            4. 启用能力后要重新生成描述文件；旧描述文件会继续返回 permission denied。
            5. 用 codesign -d --entitlements :- 检查主 App 与 .appex 都包含 com.apple.developer.networking.networkextension。
            6. 删除旧 App 和系统 VPN 配置后重装；不要直接安装 unsigned IPA，也不要用 TrollStore、免费签名或没有 Network Extension 能力的描述文件。
            """)
        }

        if ns.domain == NEVPNErrorDomain {
            switch ns.code {
            case 1:
                return TunnelError.invalid("VPN 配置无效。请确认 FarmHelperTunnel.appex 已完整嵌入并使用同一 Team 的合法描述文件签名。")
            case 2:
                return TunnelError.invalid("VPN 配置被系统禁用。请检查屏幕使用时间、MDM 或设备管理限制。")
            case 4:
                return TunnelError.invalid("VPN 配置已过期。请到 设置 › 通用 › VPN 与设备管理 删除旧配置后重试。")
            case 5:
                return TunnelError.invalid("VPN 配置无法写入。请检查钥匙串访问权限，并删除旧 App 后重新安装。")
            default:
                break
            }
        }

        return TunnelError.invalid("VPN 启动失败：\(text)（错误域 \(ns.domain) / \(ns.code)）")
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
