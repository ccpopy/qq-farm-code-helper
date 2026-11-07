import Foundation
import NetworkExtension

final class PacketTunnelProvider: NEPacketTunnelProvider {
    private var engine: TunnelEngine?
    private var socks: LocalSocks5Server?
    private var coordinator: CaptureCoordinator?
    private var monitor: ExtensionMonitor?
    private var statusTimer: DispatchSourceTimer?
    private var platform: CapturePlatform = .qq

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        Task {
            do {
                try await startInternal()
                completionHandler(nil)
            } catch {
                SharedLog.append("VPN", "启动失败：\(error.localizedDescription)", source: .tunnel)
                engine?.stop()
                engine = nil
                socks?.stop()
                socks = nil
                monitor?.stop()
                monitor = nil
                statusTimer?.cancel()
                statusTimer = nil
                completionHandler(error)
            }
        }
    }

    private func startInternal() async throws {
        let configuration = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration ?? [:]
        let platform = CapturePlatform(rawValue: configuration["platform"] as? String ?? "qq") ?? .qq
        let passthrough = configuration["passthrough"] as? Bool ?? false
        let friendCapture = configuration["friendCapture"] as? Bool ?? true
        let password = configuration["socksPassword"] as? String ?? ""
        guard password.count >= 16 else {
            throw TunnelError.invalid("本地代理凭据缺失，请重新开始获取")
        }
        if !passthrough, !CertificateAuthority.shared.exists {
            throw TunnelError.invalid("请先在应用中生成并安装抓包证书")
        }
        self.platform = platform

        let coordinator = CaptureCoordinator(platform: platform, friendCapture: friendCapture, passthrough: passthrough)
        coordinator.begin()
        self.coordinator = coordinator

        let socks = LocalSocks5Server(
            platform: platform,
            friendCapture: friendCapture,
            passthrough: passthrough,
            password: password,
            coordinator: coordinator
        )
        let socksPort = try await socks.start()
        self.socks = socks

        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "127.0.0.1")
        settings.mtu = 1500
        let ipv4 = NEIPv4Settings(addresses: ["10.231.0.1"], subnetMasks: ["255.255.255.255"])
        ipv4.includedRoutes = [NEIPv4Route.default()]
        settings.ipv4Settings = ipv4
        let ipv6 = NEIPv6Settings(addresses: ["fd71:fa42::1"], networkPrefixLengths: [128])
        ipv6.includedRoutes = [NEIPv6Route.default()]
        settings.ipv6Settings = ipv6
        settings.dnsSettings = NEDNSSettings(servers: ["223.5.5.5", "119.29.29.29"])

        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            setTunnelNetworkSettings(settings) { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            }
        }

        let engine = TunnelEngine(socksPort: socksPort, password: password, mtu: 1500, packetFlow: packetFlow)
        try engine.start()
        self.engine = engine

        let monitor = ExtensionMonitor()
        monitor.start { [weak self] in self?.engine != nil }
        self.monitor = monitor

        SharedLog.append(
            "VPN",
            "转发已就绪：全设备 TCP/UDP 进入本机 SOCKS，仅 \(UrlPolicy.farmHost) 解密；其他内容原样透传",
            source: .tunnel
        )
        startStatusTimer()
        publishStatus(ready: true)
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        statusTimer?.cancel()
        statusTimer = nil
        monitor?.stop()
        monitor = nil
        engine?.stop()
        engine = nil
        socks?.stop()
        socks = nil
        coordinator = nil
        CaptureStore.shared.markTunnelStopped()
        TunnelStatusStore.clear()
        CertificateAuthority.shared.cleanupLeafMaterial()
        SharedLog.append("VPN", "已停止抓取并释放网络；已安装的系统证书不会被自动删除", source: .tunnel)
        completionHandler()
    }

    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)? = nil) {
        let command = String(decoding: messageData, as: UTF8.self)
        switch command {
        case "clear":
            CaptureStore.shared.clear()
            completionHandler?(Data("ok".utf8))
        case "reload":
            monitor?.stop()
            monitor?.start { [weak self] in self?.engine != nil }
            completionHandler?(Data("ok".utf8))
        case "status":
            completionHandler?(statusJSON())
        default:
            completionHandler?(nil)
        }
    }

    private func startStatusTimer() {
        let timer = DispatchSource.makeTimerSource(queue: DispatchQueue.global(qos: .utility))
        timer.schedule(deadline: .now() + 1, repeating: 2)
        timer.setEventHandler { [weak self] in
            self?.publishStatus(ready: true)
        }
        timer.resume()
        statusTimer = timer
    }

    private func publishStatus(ready: Bool) {
        guard let socks, let engine else { return }
        let snapshot = socks.snapshot()
        let tunnelStats = engine.stats()
        let status = TunnelStatus(
            ready: ready,
            status: CaptureStore.shared.statusText(capturing: true, platform: platform),
            platform: platform,
            passthrough: coordinator?.passthrough ?? false,
            tlsHosts: snapshot.hosts,
            socksAccepted: snapshot.accepted,
            socksAuthenticated: snapshot.authenticated,
            socksRejected: snapshot.rejected,
            tcpPassthroughs: snapshot.passthrough,
            tunnelTxPackets: tunnelStats.txPackets,
            tunnelRxPackets: tunnelStats.rxPackets,
            updatedAt: Date()
        )
        TunnelStatusStore.save(status)
    }

    private func statusJSON() -> Data? {
        guard let status = TunnelStatusStore.load() else { return nil }
        let object: [String: Any] = [
            "ready": status.ready,
            "status": status.status,
            "platform": status.platform.rawValue,
            "connection": status.connectionText,
        ]
        return try? JSONSerialization.data(withJSONObject: object)
    }
}
