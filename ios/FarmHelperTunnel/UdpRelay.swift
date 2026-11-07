import Darwin
import Foundation

/// SOCKS5 UDP relay (RFC 1928). hev-socks5-tunnel sends UDP-in-UDP datagrams to the proxy port;
/// each client association is identified by its loopback source address.
final class UdpRelay: @unchecked Sendable {
    private let queue = DispatchQueue(label: "io.github.ccpopy.farmhelper.udp", qos: .userInitiated)
    private let lock = NSLock()
    private var listenFD: Int32 = -1
    private var source: DispatchSourceRead?
    private var active = true
    private var outgoing: [String: Outbound] = [:]
    private(set) var port: UInt16 = 0

    private final class Outbound {
        let fd: Int32
        let family: Int32
        var client: sockaddr_storage
        var clientLength: socklen_t
        var source: DispatchSourceRead?

        init(fd: Int32, family: Int32, client: sockaddr_storage, clientLength: socklen_t) {
            self.fd = fd
            self.family = family
            self.client = client
            self.clientLength = clientLength
        }
    }

    init?() {
        let fd = socket(AF_INET, SOCK_DGRAM, 0)
        guard fd >= 0 else { return nil }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = 0
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
        let bound = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bound == 0 else {
            close(fd)
            return nil
        }
        var storage = sockaddr_storage()
        var length = socklen_t(MemoryLayout<sockaddr_storage>.size)
        let named = withUnsafeMutablePointer(to: &storage) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                getsockname(fd, $0, &length)
            }
        }
        guard named == 0 else {
            close(fd)
            return nil
        }
        port = withUnsafePointer(to: &storage) { pointer in
            pointer.withMemoryRebound(to: sockaddr_in.self, capacity: 1) {
                UInt16(bigEndian: $0.pointee.sin_port)
            }
        }
        listenFD = fd
        setNonBlocking(fd)
        let readSource = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        readSource.setEventHandler { [weak self] in self?.drainIncoming() }
        readSource.resume()
        source = readSource
    }

    func stop() {
        lock.lock()
        active = false
        let fds = outgoing.values.map(\.fd)
        let sources = outgoing.values.compactMap(\.source)
        outgoing.removeAll()
        lock.unlock()
        sources.forEach { $0.cancel() }
        fds.forEach { close($0) }
        source?.cancel()
        source = nil
        if listenFD >= 0 { close(listenFD) }
        listenFD = -1
    }

    private func drainIncoming() {
        var buffer = [UInt8](repeating: 0, count: 65535)
        while active {
            var client = sockaddr_storage()
            var clientLength = socklen_t(MemoryLayout<sockaddr_storage>.size)
            let count = withUnsafeMutablePointer(to: &client) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    recvfrom(listenFD, &buffer, buffer.count, 0, $0, &clientLength)
                }
            }
            if count < 0 { break }
            guard count >= 4 else { continue }
            handle(Array(buffer[0..<count]), client: client, clientLength: clientLength)
        }
    }

    private func handle(_ datagram: [UInt8], client: sockaddr_storage, clientLength: socklen_t) {
        guard datagram[0] == 5, datagram[1] == 0, datagram[2] == 0 else { return }
        let atyp = datagram[3]
        var offset = 4
        let family: Int32
        var destination = sockaddr_storage()
        var destinationLength = socklen_t(MemoryLayout<sockaddr_storage>.size)

        switch atyp {
        case 1:
            guard datagram.count >= offset + 4 + 2 else { return }
            var address = sockaddr_in()
            address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            address.sin_family = sa_family_t(AF_INET)
            address.sin_port = 0
            address.sin_addr = in_addr(s_addr: inet_addr("0.0.0.0"))
            withUnsafeMutableBytes(of: &address.sin_addr) { raw in
                for index in 0..<4 { raw[index] = datagram[offset + index] }
            }
            offset += 4
            let port = UInt16(datagram[offset]) << 8 | UInt16(datagram[offset + 1])
            address.sin_port = port.bigEndian
            offset += 2
            withUnsafeMutableBytes(of: &destination) { target in
                withUnsafeBytes(of: address) { raw in
                    for index in 0..<MemoryLayout<sockaddr_in>.size { target[index] = raw[index] }
                }
            }
            destinationLength = socklen_t(MemoryLayout<sockaddr_in>.size)
            family = AF_INET
        case 4:
            guard datagram.count >= offset + 16 + 2 else { return }
            var address = sockaddr_in6()
            address.sin6_len = UInt8(MemoryLayout<sockaddr_in6>.size)
            address.sin6_family = sa_family_t(AF_INET6)
            address.sin6_port = 0
            withUnsafeMutableBytes(of: &address.sin6_addr) { raw in
                for index in 0..<16 { raw[index] = datagram[offset + index] }
            }
            offset += 16
            let port = UInt16(datagram[offset]) << 8 | UInt16(datagram[offset + 1])
            address.sin6_port = port.bigEndian
            offset += 2
            withUnsafeMutableBytes(of: &destination) { target in
                withUnsafeBytes(of: address) { raw in
                    for index in 0..<MemoryLayout<sockaddr_in6>.size { target[index] = raw[index] }
                }
            }
            destinationLength = socklen_t(MemoryLayout<sockaddr_in6>.size)
            family = AF_INET6
        case 3:
            guard datagram.count >= offset + 1 else { return }
            let nameLength = Int(datagram[offset])
            offset += 1
            guard datagram.count >= offset + nameLength + 2 else { return }
            let name = String(bytes: datagram[offset..<(offset + nameLength)], encoding: .utf8) ?? ""
            offset += nameLength
            let port = UInt16(datagram[offset]) << 8 | UInt16(datagram[offset + 1])
            offset += 2
            guard let resolved = resolve(name: name, port: port) else { return }
            destination = resolved.0
            destinationLength = resolved.1
            family = resolved.2
        default:
            return
        }

        guard offset < datagram.count else { return }
        let payload = Data(datagram[offset..<datagram.count])
        let key = endpointKey(family: family, client: client)
        let outbound = outboundSocket(key: key, family: family, client: client, clientLength: clientLength)
        guard let outbound else { return }
        var destinationCopy = destination
        _ = withUnsafePointer(to: &destinationCopy) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                sendto(outbound.fd, [UInt8](payload), payload.count, 0, $0, destinationLength)
            }
        }
    }

    private func outboundSocket(key: String, family: Int32, client: sockaddr_storage, clientLength: socklen_t) -> Outbound? {
        lock.lock()
        if let existing = outgoing[key] {
            lock.unlock()
            return existing
        }
        lock.unlock()

        let fd = socket(family, SOCK_DGRAM, 0)
        guard fd >= 0 else { return nil }
        setNonBlocking(fd)
        let outbound = Outbound(fd: fd, family: family, client: client, clientLength: clientLength)
        let readSource = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        readSource.setEventHandler { [weak self, weak outbound] in
            guard let self, let outbound else { return }
            self.drainOutbound(outbound)
        }
        readSource.resume()
        outbound.source = readSource

        lock.lock()
        if let existing = outgoing[key] {
            lock.unlock()
            readSource.cancel()
            close(fd)
            return existing
        }
        outgoing[key] = outbound
        lock.unlock()
        return outbound
    }

    private func drainOutbound(_ outbound: Outbound) {
        var buffer = [UInt8](repeating: 0, count: 65535)
        while active {
            var sourceAddress = sockaddr_storage()
            var sourceLength = socklen_t(MemoryLayout<sockaddr_storage>.size)
            let count = withUnsafeMutablePointer(to: &sourceAddress) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    recvfrom(outbound.fd, &buffer, buffer.count, 0, $0, &sourceLength)
                }
            }
            if count < 0 { break }
            guard count > 0 else { continue }
            var header = Data([0, 0, 0])
            if outbound.family == AF_INET {
                header.append(1)
                withUnsafeBytes(of: &sourceAddress) { raw in
                    header.append(contentsOf: raw[4..<8])
                }
            } else {
                header.append(4)
                withUnsafeBytes(of: &sourceAddress) { raw in
                    header.append(contentsOf: raw[8..<24])
                }
            }
            let sourcePort: UInt16
            if outbound.family == AF_INET {
                sourcePort = withUnsafeBytes(of: &sourceAddress) { $0.load(fromByteOffset: 2, as: UInt16.self).bigEndian }
            } else {
                sourcePort = withUnsafeBytes(of: &sourceAddress) { $0.load(fromByteOffset: 2, as: UInt16.self).bigEndian }
            }
            header.append(UInt8(sourcePort >> 8))
            header.append(UInt8(sourcePort & 0xff))
            header.append(contentsOf: buffer[0..<count])
            var client = outbound.client
            _ = withUnsafePointer(to: &client) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    sendto(listenFD, [UInt8](header), header.count, 0, $0, outbound.clientLength)
                }
            }
        }
    }

    private func endpointKey(family: Int32, client: sockaddr_storage) -> String {
        let length = family == AF_INET ? MemoryLayout<sockaddr_in>.size : MemoryLayout<sockaddr_in6>.size
        return withUnsafeBytes(of: client) { raw -> String in
            Data(raw.prefix(length)).base64EncodedString()
        }
    }

    private func resolve(name: String, port: UInt16) -> (sockaddr_storage, socklen_t, Int32)? {
        var hints = addrinfo()
        hints.ai_family = AF_UNSPEC
        hints.ai_socktype = SOCK_DGRAM
        var info: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(name, String(port), &hints, &info) == 0, let first = info else { return nil }
        defer { freeaddrinfo(info) }
        var storage = sockaddr_storage()
        let length = socklen_t(first.pointee.ai_addrlen)
        guard Int(length) <= MemoryLayout<sockaddr_storage>.size else { return nil }
        withUnsafeMutableBytes(of: &storage) { destination in
            destination.copyBytes(from: UnsafeRawBufferPointer(start: first.pointee.ai_addr, count: Int(length)))
        }
        return (storage, length, first.pointee.ai_family)
    }

    private func setNonBlocking(_ fd: Int32) {
        let flags = fcntl(fd, F_GETFL, 0)
        if flags >= 0 { _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK) }
    }
}
