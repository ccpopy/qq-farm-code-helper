import Darwin
import Foundation
import Network

final class SocksStats: @unchecked Sendable {
    private let lock = NSLock()
    private var acceptedCount = 0
    private var authenticatedCount = 0
    private var rejectedCount = 0
    private var passthroughCount = 0
    private var hosts: [String: Int] = [:]

    func accepted() {
        lock.lock(); acceptedCount += 1; lock.unlock()
    }

    func authenticated() {
        lock.lock(); authenticatedCount += 1; lock.unlock()
    }

    func rejected() {
        lock.lock(); rejectedCount += 1; lock.unlock()
    }

    func passthrough() {
        lock.lock(); passthroughCount += 1; lock.unlock()
    }

    func observeTLS(_ host: String) {
        guard !host.isEmpty, host.count <= 253,
              host.range(of: "^[a-zA-Z0-9._-]+$", options: .regularExpression) != nil else { return }
        let name = host.lowercased()
        lock.lock()
        if hosts[name] != nil || hosts.count < 40 {
            hosts[name, default: 0] += 1
        }
        lock.unlock()
    }

    func snapshot() -> (accepted: Int, authenticated: Int, rejected: Int, passthrough: Int, hosts: [String: Int]) {
        lock.lock()
        defer { lock.unlock() }
        return (acceptedCount, authenticatedCount, rejectedCount, passthroughCount, hosts)
    }
}

final class ConnectionReader {
    private let connection: NWConnection
    private var buffer = Data()

    init(_ connection: NWConnection) { self.connection = connection }

    func readExact(_ count: Int) async throws -> Data {
        while buffer.count < count {
            guard let chunk = try await connection.readChunk(maximumLength: max(4096, count)) else {
                throw TunnelError.closed
            }
            buffer.append(chunk)
        }
        let result = buffer.prefix(count)
        buffer.removeFirst(count)
        return Data(result)
    }

    func takeBuffered() -> Data {
        let data = buffer
        buffer.removeAll()
        return data
    }

    struct HelloData {
        let result: TlsHelloResult
        let consumed: Data
        let extra: Data
    }

    func readHello(maximum: Int = 65536) async throws -> HelloData {
        var data = takeBuffered()
        while true {
            let parsed = TlsHelloParser.parse(data)
            switch parsed {
            case .needMore:
                if data.count >= maximum {
                    return HelloData(result: .notTLS(consumed: data.count), consumed: data, extra: Data())
                }
                guard let chunk = try await connection.readChunk(maximumLength: 16384) else {
                    throw TunnelError.closed
                }
                data.append(chunk)
            case .notTLS(let consumed):
                let count = min(max(consumed, 0), data.count)
                return HelloData(
                    result: parsed,
                    consumed: data.subdata(in: 0..<count),
                    extra: data.subdata(in: count..<data.count)
                )
            case .parsed(let consumed, _):
                let count = min(max(consumed, 0), data.count)
                return HelloData(
                    result: parsed,
                    consumed: data.subdata(in: 0..<count),
                    extra: data.subdata(in: count..<data.count)
                )
            }
        }
    }
}

final class Socks5Session: @unchecked Sendable {
    private let client: NWConnection
    private let reader: ConnectionReader
    private let password: String
    private let platform: CapturePlatform
    private let friendCapture: Bool
    private let passthrough: Bool
    private let coordinator: CaptureCoordinator
    private let udpPort: UInt16
    private let stats: SocksStats
    private let queue: DispatchQueue
    private let generation: Int
    private var handedOff = false

    init(
        connection: NWConnection,
        password: String,
        platform: CapturePlatform,
        friendCapture: Bool,
        passthrough: Bool,
        coordinator: CaptureCoordinator,
        udpPort: UInt16,
        stats: SocksStats,
        queue: DispatchQueue
    ) {
        self.client = connection
        self.reader = ConnectionReader(connection)
        self.password = password
        self.platform = platform
        self.friendCapture = friendCapture
        self.passthrough = passthrough
        self.coordinator = coordinator
        self.udpPort = udpPort
        self.stats = stats
        self.queue = queue
        self.generation = CaptureStore.shared.generation()
    }

    func run() async {
        do {
            try await handshake()
            if !handedOff { client.cancel() }
        } catch let error as TunnelError {
            if case .closed = error {
                // Normal client shutdown.
            } else {
                SharedLog.append("代理", "连接未完成：\(error.localizedDescription)；不会绕过证书校验", source: .tunnel)
            }
            client.cancel()
        } catch {
            SharedLog.append("代理", "连接未完成：\(error.localizedDescription)；不会绕过证书校验", source: .tunnel)
            client.cancel()
        }
    }

    private func handshake() async throws {
        let greeting = try await reader.readExact(2)
        guard greeting[0] == 5, greeting[1] > 0 else { return }
        let methods = try await reader.readExact(Int(greeting[1]))
        let authentication = methods.contains(2)
        try await send(Data([5, authentication ? 2 : 0xff]))
        guard authentication else {
            stats.rejected()
            return
        }

        let authHeader = try await reader.readExact(2)
        guard authHeader[0] == 1 else { return }
        let user = try await reader.readExact(Int(authHeader[1]))
        let passwordLength = try await reader.readExact(1)[0]
        let presented = try await reader.readExact(Int(passwordLength))
        let ok = String(decoding: user, as: UTF8.self) == "farm"
            && String(decoding: presented, as: UTF8.self) == password
        try await send(Data([1, ok ? 0 : 1]))
        guard ok else {
            stats.rejected()
            return
        }
        stats.authenticated()
        try await request()
    }

    private func request() async throws {
        let header = try await reader.readExact(4)
        guard header[0] == 5, header[2] == 0 else { return }
        let command = header[1]
        let addressType = header[3]

        var host = ""
        switch addressType {
        case 1:
            host = Self.ipv4String(try await reader.readExact(4))
        case 4:
            host = Self.ipv6String(try await reader.readExact(16))
        case 3:
            let length = try await reader.readExact(1)[0]
            host = String(decoding: try await reader.readExact(Int(length)), as: UTF8.self)
        default:
            try await send(reply(code: 8))
            return
        }
        let portBytes = try await reader.readExact(2)
        let port = UInt16(portBytes[0]) << 8 | UInt16(portBytes[1])

        if command == 3 {
            try await send(reply(code: 0, port: udpPort))
            _ = try? await drainControl()
            return
        }
        guard command == 1 else {
            try await send(reply(code: 7))
            return
        }
        guard port > 0, !host.isEmpty, host != "0.0.0.0", host != "::" else {
            try await send(reply(code: 1))
            return
        }

        let upstream = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!, using: .tcp)
        do {
            try await upstream.prepareAndWaitReady(timeout: 10, queue: queue)
        } catch {
            upstream.cancel()
            stats.rejected()
            try? await send(reply(code: 4))
            return
        }
        try await send(reply(code: 0))

        if passthrough || port != 443 {
            stats.passthrough()
            handedOff = true
            BidirectionalRelay(a: client, b: upstream).start()
            return
        }

        let hello = try await reader.readHello()
        switch hello.result {
        case .parsed(_, let serverName):
            stats.observeTLS(serverName)
            if serverName.caseInsensitiveCompare(UrlPolicy.farmHost) != .orderedSame {
                var initial = hello.consumed
                initial.append(hello.extra)
                try await upstream.sendAsync(initial)
                stats.passthrough()
                handedOff = true
                BidirectionalRelay(a: client, b: upstream).start()
                return
            }
            upstream.cancel()
            handedOff = true
            let handler = TlsMitmHandler(
                platform: platform,
                friendCapture: friendCapture,
                coordinator: coordinator,
                queue: queue
            )
            await handler.handle(client: client, consumed: hello.consumed, generation: generation)
        case .notTLS:
            var initial = hello.consumed
            initial.append(hello.extra)
            try await upstream.sendAsync(initial)
            stats.passthrough()
            handedOff = true
            BidirectionalRelay(a: client, b: upstream).start()
        case .needMore:
            upstream.cancel()
            throw TunnelError.invalid("TLS ClientHello 不完整")
        }
    }

    private func drainControl() async throws {
        while true {
            guard try await client.readChunk(maximumLength: 1024) != nil else { return }
        }
    }

    private func send(_ data: Data) async throws {
        try await client.sendAsync(data)
    }

    private func reply(code: UInt8, port: UInt16 = 0) -> Data {
        var data = Data([5, code, 0, 1])
        if code == 0 {
            data.append(contentsOf: [127, 0, 0, 1])
        } else {
            data.append(contentsOf: [0, 0, 0, 0])
        }
        data.append(UInt8(port >> 8))
        data.append(UInt8(port & 0xff))
        return data
    }

    private static func ipv4String(_ data: Data) -> String {
        data.map { String($0) }.joined(separator: ".")
    }

    private static func ipv6String(_ data: Data) -> String {
        var address = in6_addr()
        withUnsafeMutableBytes(of: &address) { $0.copyBytes(from: data) }
        var buffer = [CChar](repeating: 0, count: Int(INET6_ADDRSTRLEN))
        var copy = address
        guard inet_ntop(AF_INET6, &copy, &buffer, socklen_t(buffer.count)) != nil else { return "" }
        return String(cString: buffer)
    }
}
