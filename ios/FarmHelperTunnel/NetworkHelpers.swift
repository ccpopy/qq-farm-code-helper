import Foundation
import Network

final class ContinuationGate<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<T, Error>?

    init(_ continuation: CheckedContinuation<T, Error>) {
        self.continuation = continuation
    }

    func succeed(_ value: T) {
        lock.lock()
        let continuation = self.continuation
        self.continuation = nil
        lock.unlock()
        continuation?.resume(returning: value)
    }

    func fail(_ error: Error) {
        lock.lock()
        let continuation = self.continuation
        self.continuation = nil
        lock.unlock()
        continuation?.resume(throwing: error)
    }
}

extension NWConnection {
    func prepareAndWaitReady(timeout: TimeInterval = 10, queue: DispatchQueue) async throws {
        if state == .ready { return }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            let gate = ContinuationGate(continuation)
            stateUpdateHandler = { state in
                switch state {
                case .ready:
                    gate.succeed(())
                case .failed(let error):
                    gate.fail(error)
                case .cancelled:
                    gate.fail(TunnelError.closed)
                case .waiting(let error):
                    // TLS verification failures land here and never become ready.
                    if case .tls = error {
                        gate.fail(TunnelError.tls(error.localizedDescription))
                    }
                default:
                    break
                }
            }
            if state == .setup { start(queue: queue) }
            queue.asyncAfter(deadline: .now() + timeout) { gate.fail(TunnelError.timeout) }
        }
    }

    func readChunk(maximumLength: Int = 65536) async throws -> Data? {
        while true {
            let result: (Data?, Bool) = try await withCheckedThrowingContinuation { continuation in
                receive(minimumIncompleteLength: 1, maximumLength: maximumLength) { data, _, isComplete, error in
                    if let error {
                        continuation.resume(throwing: error)
                    } else {
                        continuation.resume(returning: (data, isComplete))
                    }
                }
            }
            if let data = result.0, !data.isEmpty { return data }
            if result.1 { return nil }
        }
    }

    func sendAsync(_ data: Data) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            send(content: data, contentContext: .defaultMessage, isComplete: false, completion: .contentProcessed { error in
                if let error {
                    continuation.resume(throwing: error)
                } else {
                    continuation.resume(returning: ())
                }
            })
        }
    }

    func sendEOF() {
        send(content: nil, contentContext: .finalMessage, isComplete: true, completion: .contentProcessed { _ in })
    }
}

extension NWListener {
    func prepareAndWaitReady(timeout: TimeInterval = 5, queue: DispatchQueue) async throws -> UInt16 {
        if state != .ready {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                let gate = ContinuationGate(continuation)
                stateUpdateHandler = { state in
                    switch state {
                    case .ready: gate.succeed(())
                    case .failed(let error): gate.fail(error)
                    case .cancelled: gate.fail(TunnelError.closed)
                    default: break
                    }
                }
                if state == .setup { start(queue: queue) }
                queue.asyncAfter(deadline: .now() + timeout) { gate.fail(TunnelError.timeout) }
            }
        }
        guard let port = port else { throw TunnelError.invalid("本地端口分配失败") }
        return port.rawValue
    }
}

func readHTTPHeaders(_ connection: NWConnection, maximum: Int = 32768) async throws -> (headers: Data, extra: Data) {
    var buffer = Data()
    let marker = Data("\r\n\r\n".utf8)
    while buffer.count < maximum {
        guard let chunk = try await connection.readChunk(maximumLength: min(8192, maximum)) else {
            throw TunnelError.closed
        }
        buffer.append(chunk)
        if let range = buffer.range(of: marker) {
            return (
                buffer.subdata(in: 0..<range.upperBound),
                buffer.subdata(in: range.upperBound..<buffer.count)
            )
        }
    }
    throw TunnelError.invalid("HTTP 头超过安全限制")
}

/// Pumps bytes in both directions. Once either direction ends, both sockets are cancelled and
/// the finish callback runs exactly once.
final class BidirectionalRelay: @unchecked Sendable {
    private let a: NWConnection
    private let b: NWConnection
    private let onAToB: ((Data) -> Void)?
    private let onBToA: ((Data) -> Void)?
    private let onFinish: (() -> Void)?
    private let lock = NSLock()
    private var finished = false

    init(
        a: NWConnection,
        b: NWConnection,
        onAToB: ((Data) -> Void)? = nil,
        onBToA: ((Data) -> Void)? = nil,
        onFinish: (() -> Void)? = nil
    ) {
        self.a = a
        self.b = b
        self.onAToB = onAToB
        self.onBToA = onBToA
        self.onFinish = onFinish
    }

    func start() {
        // The tasks retain the relay until both directions finish; without this the local
        // relay object would deallocate as soon as the caller returns.
        Task.detached {
            await self.pump(from: self.a, to: self.b, observe: self.onAToB)
        }
        Task.detached {
            await self.pump(from: self.b, to: self.a, observe: self.onBToA)
        }
    }

    private func pump(from source: NWConnection, to destination: NWConnection, observe: ((Data) -> Void)?) async {
        do {
            while let chunk = try await source.readChunk() {
                try await destination.sendAsync(chunk)
                observe?(chunk)
            }
        } catch {
            // A half-open connection is normal here; the other direction may still drain.
        }
        destination.sendEOF()
        finish()
    }

    private func finish() {
        lock.lock()
        if finished {
            lock.unlock()
            return
        }
        finished = true
        lock.unlock()
        a.cancel()
        b.cancel()
        onFinish?()
    }

    func stop() {
        lock.lock()
        if finished {
            lock.unlock()
            return
        }
        finished = true
        lock.unlock()
        a.cancel()
        b.cancel()
    }
}
