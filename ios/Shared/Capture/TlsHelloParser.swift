import Foundation

enum TlsHelloResult: Equatable {
    case needMore
    case notTLS(consumed: Int)
    case parsed(consumed: Int, serverName: String)
}

/// Inspect only plaintext SNI, preserving bytes for either TLS or opaque passthrough.
enum TlsHelloParser {
    static func parse(_ data: Data) -> TlsHelloResult {
        let bytes = [UInt8](data)
        var offset = 0
        var handshake: [UInt8] = []

        while offset < bytes.count {
            guard bytes.count - offset >= 5 else { return .needMore }
            let type = bytes[offset]
            let major = bytes[offset + 1]
            guard type == 22, major == 3 else { return .notTLS(consumed: offset + 5) }
            let length = Int(bytes[offset + 3]) << 8 | Int(bytes[offset + 4])
            guard length <= 18432, offset + 5 + length <= 65536 else {
                return .notTLS(consumed: offset + 5)
            }
            guard bytes.count - offset - 5 >= length else { return .needMore }
            handshake.append(contentsOf: bytes[(offset + 5)..<(offset + 5 + length)])
            offset += 5 + length

            guard handshake.count >= 4 else { continue }
            guard handshake[0] == 1 else { return .notTLS(consumed: offset) }
            let total = Int(handshake[1]) << 16 | Int(handshake[2]) << 8 | Int(handshake[3])
            guard total <= 65532 else { return .notTLS(consumed: offset) }
            if handshake.count >= total + 4 {
                let name = (try? parseName(handshake)) ?? ""
                return .parsed(consumed: offset, serverName: name)
            }
        }
        return .needMore
    }

    private static func parseName(_ data: [UInt8]) throws -> String {
        var p = 4 + 2 + 32
        guard p < data.count else { throw TlsParseError.malformed }
        p += 1 + Int(data[p])
        guard p + 2 <= data.count else { throw TlsParseError.malformed }
        p += 2 + u16(data, p)
        guard p < data.count else { throw TlsParseError.malformed }
        p += 1 + Int(data[p])
        if p == data.count { return "" }
        guard p + 2 <= data.count else { throw TlsParseError.malformed }
        let extensionsEnd = p + 2 + u16(data, p)
        p += 2
        guard extensionsEnd <= data.count else { throw TlsParseError.malformed }

        while p + 4 <= extensionsEnd {
            let type = u16(data, p)
            let length = u16(data, p + 2)
            p += 4
            guard p + length <= extensionsEnd else { throw TlsParseError.malformed }
            if type == 0 {
                guard p + 2 <= data.count else { throw TlsParseError.malformed }
                let listEnd = p + 2 + u16(data, p)
                var q = p + 2
                guard listEnd <= p + length, listEnd <= data.count else { throw TlsParseError.malformed }
                while q + 3 <= listEnd {
                    let kind = data[q]
                    let size = u16(data, q + 1)
                    q += 3
                    guard q + size <= listEnd else { throw TlsParseError.malformed }
                    if kind == 0 {
                        return String(bytes: data[q..<(q + size)], encoding: .ascii) ?? ""
                    }
                    q += size
                }
            }
            p += length
        }
        return ""
    }

    private static func u16(_ data: [UInt8], _ index: Int) -> Int {
        Int(data[index]) * 256 + Int(data[index + 1])
    }

    private enum TlsParseError: Error { case malformed }
}
