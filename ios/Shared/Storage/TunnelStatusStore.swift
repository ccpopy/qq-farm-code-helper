import Foundation

struct TunnelStatus: Codable, Equatable, Sendable {
    var ready: Bool
    var status: String
    var platform: CapturePlatform
    var passthrough: Bool
    var tlsHosts: [String: Int]
    var socksAccepted: Int
    var socksAuthenticated: Int
    var socksRejected: Int
    var tcpPassthroughs: Int
    var tunnelTxPackets: UInt64
    var tunnelRxPackets: UInt64
    var updatedAt: Date

    var connectionText: String {
        var lines: [String] = []
        lines.append("本次接管：\(platform.label)\(passthrough ? " · 全量透传诊断" : "")")
        lines.append("SOCKS 接入 / 认证：\(socksAccepted) / \(socksAuthenticated) · 拒绝：\(socksRejected)")
        lines.append("TCP 透传：\(tcpPassthroughs) · TUN 发 / 收：\(tunnelTxPackets) / \(tunnelRxPackets)")
        if tlsHosts.isEmpty {
            lines.append("尚未观察到新的 TLS 握手；已建立的连接不会补出登录请求。")
        } else {
            lines.append("TLS 主机（仅本次内存；不含路径、参数和内容）：")
            for (host, count) in tlsHosts.sorted(by: { $0.key < $1.key }) {
                lines.append("\(host) × \(count)")
            }
        }
        return lines.joined(separator: "\n")
    }
}

enum TunnelStatusStore {
    private static let key = "tunnel.status.v1"
    private static let lock = NSLock()

    static func save(_ status: TunnelStatus) {
        lock.lock()
        defer { lock.unlock() }
        if let data = try? JSONEncoder().encode(status) {
            AppGroup.defaults.set(data, forKey: key)
        }
    }

    static func load() -> TunnelStatus? {
        lock.lock()
        defer { lock.unlock() }
        guard let data = AppGroup.defaults.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(TunnelStatus.self, from: data)
    }

    static func clear() {
        lock.lock()
        AppGroup.defaults.removeObject(forKey: key)
        lock.unlock()
    }
}
