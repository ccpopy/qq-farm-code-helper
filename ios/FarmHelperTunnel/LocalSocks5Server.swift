import Foundation
import Network

final class LocalSocks5Server: @unchecked Sendable {
    private let password: String
    private let platform: CapturePlatform
    private let friendCapture: Bool
    private let passthrough: Bool
    private let coordinator: CaptureCoordinator
    private let queue = DispatchQueue(label: "io.github.ccpopy.farmhelper.socks", qos: .userInitiated)
    private var listener: NWListener?
    private var udp: UdpRelay?
    private var active = false

    let stats = SocksStats()
    private(set) var port: UInt16 = 0
    private(set) var udpPort: UInt16 = 0

    init(platform: CapturePlatform, friendCapture: Bool, passthrough: Bool, password: String, coordinator: CaptureCoordinator) {
        self.platform = platform
        self.friendCapture = friendCapture
        self.passthrough = passthrough
        self.password = password
        self.coordinator = coordinator
    }

    func start() async throws -> UInt16 {
        guard let udp = UdpRelay() else {
            throw TunnelError.socket("UDP 转发端口创建失败")
        }
        self.udp = udp
        udpPort = udp.port

        let parameters = NWParameters.tcp
        parameters.allowLocalEndpointReuse = true
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        let listener = try NWListener(using: parameters)
        listener.newConnectionHandler = { [weak self] connection in
            guard let self, self.active else {
                connection.cancel()
                return
            }
            self.stats.accepted()
            connection.start(queue: self.queue)
            let session = Socks5Session(
                connection: connection,
                password: self.password,
                platform: self.platform,
                friendCapture: self.friendCapture,
                passthrough: self.passthrough,
                coordinator: self.coordinator,
                udpPort: self.udpPort,
                stats: self.stats,
                queue: self.queue
            )
            Task { await session.run() }
        }
        self.listener = listener
        active = true
        port = try await listener.prepareAndWaitReady(timeout: 5, queue: queue)
        SharedLog.append(
            "代理",
            "本地 SOCKS5 已就绪（仅回环，用户名 farm）；UDP 关联端口 \(udpPort)",
            source: .tunnel
        )
        return port
    }

    func stop() {
        active = false
        listener?.cancel()
        listener = nil
        udp?.stop()
        udp = nil
    }

    func snapshot() -> (accepted: Int, authenticated: Int, rejected: Int, passthrough: Int, hosts: [String: Int]) {
        stats.snapshot()
    }
}
