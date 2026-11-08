import Foundation
import Network

/// Terminates the farm's TLS inside the tunnel extension. The upstream certificate is always
/// authenticated by the system trust store first; the local leaf copies only its subject, DNS
/// SANs and validity. No HTTP login request is sent upstream when Code-only mode is active.
final class TlsMitmHandler: @unchecked Sendable {
    private let platform: CapturePlatform
    private let friendCapture: Bool
    private let coordinator: CaptureCoordinator
    private let queue: DispatchQueue

    init(platform: CapturePlatform, friendCapture: Bool, coordinator: CaptureCoordinator, queue: DispatchQueue) {
        self.platform = platform
        self.friendCapture = friendCapture
        self.coordinator = coordinator
        self.queue = queue
    }

    func handle(client: NWConnection, consumed: Data, generation: Int) async {
        let host = UrlPolicy.farmHost
        var upstream: NWConnection?
        do {
            SharedLog.append("证书", "正在校验真实农场上游证书", source: .tunnel)
            let (connection, leafDER) = try await TLSClient.connect(host: host, port: 443, queue: queue)
            upstream = connection
            let material = try CertificateAuthority.shared.leafIdentity(remoteDER: leafDER, host: host)
            SharedLog.append("证书", "已使用校验通过的上游证书生成兼容证书：Subject、DNS SAN、有效期", source: .tunnel)

            let session = try await TlsBridge.open(
                identity: material.identity,
                chain: material.certificates,
                client: client,
                consumed: consumed,
                queue: queue
            )
            let decrypted = session.connection

            let request: (headers: Data, extra: Data)
            do {
                request = try await readHTTPHeaders(decrypted)
            } catch {
                SharedLog.append("证书", "客户端 TLS 握手失败：\(error.localizedDescription)", source: .tunnel)
                throw error
            }
            SharedLog.append("证书", "农场目标连接的 TLS 握手已验证成功", source: .tunnel)

            let inspection = inspect(request.headers, generation: generation)
            if inspection.codeOnly {
                let response = Data("HTTP/1.1 451 Unavailable For Legal Reasons\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".utf8)
                try? await decrypted.sendAsync(response)
                coordinator.finish(generation: generation)
                SharedLog.append("抓取", "只获取 Code：已阻止本次官方登录先消费 Code", source: .tunnel)
                upstream?.cancel()
                _ = session
                return
            }

            let sanitized = sanitize(request.headers)
            try await upstream!.sendAsync(sanitized)
            if !request.extra.isEmpty {
                try await upstream!.sendAsync(request.extra)
            }

            let response = try await readHTTPHeaders(upstream!)
            let responseText = String(decoding: response.headers, as: UTF8.self)
            try await decrypted.sendAsync(response.headers)

            let inspector = WsInspector()
            var lastRevision = 0
            func feed(_ data: Data) {
                guard inspection.owned else { return }
                inspector.feed([UInt8](data))
                guard inspector.revision != lastRevision else { return }
                lastRevision = inspector.revision
                self.coordinator.merge(
                    generation: generation,
                    ownGid: inspector.ownGid,
                    gids: inspector.pendingGids,
                    friendsObserved: inspector.friendsObserved,
                    complete: inspector.friendsComplete
                )
            }

            if !response.extra.isEmpty {
                try await decrypted.sendAsync(response.extra)
                feed(response.extra)
            }

            if inspection.owned {
                if !responseText.hasPrefix("HTTP/1.1 101 ") {
                    SharedLog.append("抓取", "农场登录未返回 WebSocket 101；本次结果已保留，未继续读取", source: .tunnel)
                    coordinator.finish(generation: generation)
                    upstream?.cancel()
                    decrypted.cancel()
                    _ = session
                    return
                }
                SharedLog.append("抓取", "已连接农场，收到的 Code 和好友可立即同步；VPN 保持运行，清除结果可继续换号获取", source: .tunnel)
            }

            let relay = BidirectionalRelay(
                a: decrypted,
                b: upstream!,
                onBToA: { data in
                    feed(data)
                },
                onFinish: {
                    if inspection.owned {
                        self.coordinator.finish(generation: generation)
                    }
                    _ = session
                }
            )
            relay.start()
        } catch {
            SharedLog.append("代理", "农场目标连接未完成：\(error.localizedDescription)；不会绕过证书校验", source: .tunnel)
            coordinator.finish(generation: generation)
            upstream?.cancel()
            client.cancel()
        }
    }

    private func inspect(_ headers: Data, generation: Int) -> (owned: Bool, codeOnly: Bool) {
        let text = String(decoding: headers, as: UTF8.self)
        let lines = text.components(separatedBy: "\r\n")
        let first = lines.first?.split(separator: " ").map(String.init) ?? []
        guard first.count == 3, first[0] == "GET", header("upgrade", lines).lowercased() == "websocket" else {
            return (false, false)
        }
        guard let code = try? UrlPolicy.captureCode(
            requestTarget: first[1],
            host: UrlPolicy.hostWithoutPort(header("host", lines))
        ) else {
            return (false, false)
        }
        guard coordinator.publish(code: code, generation: generation) else {
            return (false, false)
        }
        let friendsOnly = platform == .wx && friendCapture
        let codeOnly = !friendsOnly && !friendCapture
        return (true, codeOnly)
    }

    private func sanitize(_ headers: Data) -> Data {
        let text = String(decoding: headers, as: UTF8.self)
        let cleaned = text.replacingOccurrences(
            of: "(?im)^Sec-WebSocket-Extensions:[^\\r\\n]*\\r\\n",
            with: "",
            options: .regularExpression
        )
        return Data(cleaned.utf8)
    }

    private func header(_ name: String, _ lines: [String]) -> String {
        var result = ""
        for line in lines {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let key = String(line[..<colon]).trimmingCharacters(in: .whitespaces)
            guard key.caseInsensitiveCompare(name) == .orderedSame else { continue }
            if !result.isEmpty { return "" }
            result = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
        }
        return result
    }
}
