import Foundation
import Network
import Security

/// Authenticates the real farm gateway with the system trust store and returns its leaf DER.
enum TLSClient {
    static func connect(host: String, port: UInt16 = 443, queue: DispatchQueue) async throws -> (connection: NWConnection, leafDER: Data) {
        let tls = NWProtocolTLS.Options()
        sec_protocol_options_set_tls_server_name(tls.securityProtocolOptions, host)
        sec_protocol_options_add_tls_application_protocol(tls.securityProtocolOptions, "http/1.1")
        let parameters = NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
        let connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!, using: parameters)
        do {
            try await connection.prepareAndWaitReady(timeout: 10, queue: queue)
        } catch {
            connection.cancel()
            throw TunnelError.tls("上游农场网关校验失败：\(error.localizedDescription)")
        }
        guard let metadata = connection.metadata(definition: NWProtocolTLS.definition) as? NWProtocolTLS.Metadata else {
            connection.cancel()
            throw TunnelError.tls("未取得上游 TLS 元数据")
        }
        var leaf: Data?
        sec_protocol_metadata_access_peer_certificate_chain(metadata.securityProtocolMetadata) { certificate in
            guard leaf == nil else { return }
            guard let reference = sec_certificate_copy_ref(certificate)?.takeRetainedValue() else { return }
            leaf = SecCertificateCopyData(reference) as Data
        }
        guard let leaf else {
            connection.cancel()
            throw TunnelError.tls("未取得上游证书")
        }
        return (connection, leaf)
    }
}

/// Bridges an already-read ClientHello into a local TLS server.
///
/// Public iOS TLS APIs do not expose a socket whose consumed ClientHello can be replayed, so we
/// mirror the Android approach: a loopback TCP pair carries the raw TLS bytes, and the accepted
/// side is the decrypted stream. The client's original connection is pumped into the plain end.
final class TlsBridgeSession {
    let connection: NWConnection
    private let listener: NWListener
    private let plain: NWConnection
    private let relay: BidirectionalRelay

    init(connection: NWConnection, listener: NWListener, plain: NWConnection, relay: BidirectionalRelay) {
        self.connection = connection
        self.listener = listener
        self.plain = plain
        self.relay = relay
    }

    deinit {
        listener.cancel()
        relay.stop()
        connection.cancel()
    }
}

final class AcceptGate: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<NWConnection, Error>?
    private var pending: NWConnection?
    private var failure: Error?

    func accept(_ connection: NWConnection) {
        lock.lock()
        if let continuation {
            self.continuation = nil
            lock.unlock()
            continuation.resume(returning: connection)
        } else {
            pending = connection
            lock.unlock()
        }
    }

    func fail(_ error: Error) {
        lock.lock()
        if let continuation {
            self.continuation = nil
            lock.unlock()
            continuation.resume(throwing: error)
        } else {
            failure = error
            lock.unlock()
        }
    }

    func wait(timeout: TimeInterval) async throws -> NWConnection {
        lock.lock()
        if let pending {
            self.pending = nil
            lock.unlock()
            return pending
        }
        if let failure {
            self.failure = nil
            lock.unlock()
            throw failure
        }
        lock.unlock()
        return try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            self.continuation = continuation
            lock.unlock()
            DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { [weak self] in
                self?.fail(TunnelError.timeout)
            }
        }
    }
}

enum TlsBridge {
    static func open(
        identity: SecIdentity,
        chain: [SecCertificate],
        client: NWConnection,
        consumed: Data,
        queue: DispatchQueue
    ) async throws -> TlsBridgeSession {
        let tls = NWProtocolTLS.Options()
        let secIdentity = sec_identity_create_with_certificates(identity, (chain as NSArray) as CFArray)
            ?? sec_identity_create(identity)
        guard let secIdentity else { throw TunnelError.invalid("无法创建 TLS 身份") }
        sec_protocol_options_set_local_identity(tls.securityProtocolOptions, secIdentity)
        sec_protocol_options_add_tls_application_protocol(tls.securityProtocolOptions, "http/1.1")

        let parameters = NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
        parameters.allowLocalEndpointReuse = true
        parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
        let listener = try NWListener(using: parameters)

        let gate = AcceptGate()
        listener.newConnectionHandler = { connection in
            connection.start(queue: queue)
            gate.accept(connection)
        }
        let port = try await listener.prepareAndWaitReady(timeout: 5, queue: queue)
        let plain = NWConnection(host: "127.0.0.1", port: NWEndpoint.Port(rawValue: port)!, using: .tcp)
        try await plain.prepareAndWaitReady(timeout: 5, queue: queue)
        let accepted = try await gate.wait(timeout: 5)

        // Replay the ClientHello already read from the client, then keep the raw pipe open.
        try await plain.sendAsync(consumed)
        let relay = BidirectionalRelay(a: client, b: plain)
        relay.start()
        return TlsBridgeSession(connection: accepted, listener: listener, plain: plain, relay: relay)
    }
}
