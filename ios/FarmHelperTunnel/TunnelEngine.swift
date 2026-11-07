import Darwin
import Foundation
import HevSocks5Tunnel
import NetworkExtension

/// Bridges NEPacketTunnelFlow packets to hev-socks5-tunnel through a socketpair. hev's Apple
/// tunnel handler expects a 4-byte big-endian address family before each IP packet.
final class TunnelEngine: @unchecked Sendable {
    private let packetFlow: NEPacketTunnelFlow
    private let config: Data
    private let queue = DispatchQueue(label: "io.github.ccpopy.farmhelper.tunnel", qos: .userInitiated)
    private var tunFD: Int32 = -1
    private var bridgeFD: Int32 = -1
    private var source: DispatchSourceRead?
    private var running = false
    private var pendingInput = 0
    private let maxPendingInput = 4096

    init(socksPort: UInt16, password: String, mtu: Int, packetFlow: NEPacketTunnelFlow) {
        self.packetFlow = packetFlow
        let text = """
        tunnel:
          name: tun0
          mtu: \(mtu)
          ipv4: 10.231.0.1
          ipv6: 'fd71:fa42::1'
        socks5:
          address: 127.0.0.1
          port: \(socksPort)
          username: farm
          password: '\(password)'
          udp: udp
        misc:
          log-level: error
          log-file: null
          task-stack-size: 36864
          tcp-buffer-size: 16384
          udp-recv-buffer-size: 524288
          udp-copy-buffer-nums: 20
          max-session-count: 1200
          connect-timeout: 10
          tcp-read-write-timeout: 86400
          udp-read-write-timeout: 86400
        """
        self.config = Data(text.utf8)
    }

    func start() throws {
        var descriptors = [Int32](repeating: -1, count: 2)
        guard socketpair(AF_UNIX, SOCK_DGRAM, 0, &descriptors) == 0 else {
            throw TunnelError.socket("socketpair 创建失败")
        }
        tunFD = descriptors[0]
        bridgeFD = descriptors[1]
        tuneSocket(tunFD)
        tuneSocket(bridgeFD)
        setNonBlocking(bridgeFD)
        running = true
        startReadLoop()
        startPacketRead()

        let config = self.config
        let fd = self.tunFD
        Thread.detachNewThread { [weak self] in
            let result = config.withUnsafeBytes { raw -> Int32 in
                guard let base = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return -1 }
                return hev_socks5_tunnel_main_from_str(base, UInt32(raw.count), fd)
            }
            self?.running = false
            SharedLog.append("VPN", "转发核心已退出（\(result)）", source: .tunnel)
        }
    }

    func stop() {
        guard running || tunFD >= 0 else { return }
        running = false
        hev_socks5_tunnel_quit()
        source?.cancel()
        source = nil
        if bridgeFD >= 0 { close(bridgeFD) }
        if tunFD >= 0 { close(tunFD) }
        bridgeFD = -1
        tunFD = -1
    }

    func stats() -> (txPackets: UInt64, txBytes: UInt64, rxPackets: UInt64, rxBytes: UInt64) {
        var values = [Int](repeating: 0, count: 4)
        hev_socks5_tunnel_stats(&values[0], &values[1], &values[2], &values[3])
        return (UInt64(values[0]), UInt64(values[1]), UInt64(values[2]), UInt64(values[3]))
    }

    private func startPacketRead() {
        packetFlow.readPackets { [weak self] packets, protocols in
            guard let self, self.running else { return }
            for (index, packet) in packets.enumerated() {
                let family = index < protocols.count ? protocols[index].int32Value : AF_INET
                guard family == AF_INET || family == AF_INET6 else { continue }
                if self.pendingInput >= self.maxPendingInput { continue }
                self.pendingInput += 1
                self.writePacket(packet, family: family)
                self.pendingInput = max(0, self.pendingInput - 1)
            }
            if self.running { self.startPacketRead() }
        }
    }

    private func writePacket(_ packet: Data, family: Int32) {
        guard bridgeFD >= 0 else { return }
        var header = UInt32(bitPattern: family).bigEndian
        let result = withUnsafeMutablePointer(to: &header) { headerPointer in
            packet.withUnsafeBytes { bytes -> Int in
                guard let base = bytes.baseAddress else { return -1 }
                var vectors = [
                    iovec(iov_base: UnsafeMutableRawPointer(headerPointer), iov_len: MemoryLayout<UInt32>.size),
                    iovec(iov_base: UnsafeMutableRawPointer(mutating: base), iov_len: bytes.count),
                ]
                return writev(bridgeFD, &vectors, 2)
            }
        }
        if result <= 0, errno != EAGAIN {
            SharedLog.append("VPN", "写入转发核心失败（errno \(errno)）", source: .tunnel)
        }
    }

    private func startReadLoop() {
        let readSource = DispatchSource.makeReadSource(fileDescriptor: bridgeFD, queue: queue)
        readSource.setEventHandler { [weak self] in self?.drainOutput() }
        readSource.resume()
        source = readSource
    }

    private func drainOutput() {
        var buffer = [UInt8](repeating: 0, count: 65536)
        while running {
            let count = read(bridgeFD, &buffer, buffer.count)
            if count > 4 {
                let family = buffer.withUnsafeBytes { $0.load(as: UInt32.self).bigEndian }
                let payload = Data(buffer[4..<count])
                let protocolNumber = NSNumber(value: Int32(bitPattern: family))
                packetFlow.writePackets([payload], withProtocols: [protocolNumber])
                continue
            }
            if count == 0 { break }
            if count < 0, errno == EAGAIN { break }
            break
        }
    }

    private func tuneSocket(_ fd: Int32) {
        var size: Int32 = 4 * 1024 * 1024
        _ = setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &size, socklen_t(MemoryLayout<Int32>.size))
        _ = setsockopt(fd, SOL_SOCKET, SO_RCVBUF, &size, socklen_t(MemoryLayout<Int32>.size))
    }

    private func setNonBlocking(_ fd: Int32) {
        let flags = fcntl(fd, F_GETFL, 0)
        if flags >= 0 { _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK) }
    }
}
