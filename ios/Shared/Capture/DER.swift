import Foundation

/// Narrow DER encoder/parser for our RSA/SHA-256 root and DNS-only leaves.
enum DER {
    static func tlv(_ tag: UInt8, _ body: Data) -> Data {
        var out = Data([tag])
        let length = body.count
        if length < 128 {
            out.append(UInt8(length))
        } else {
            var value = length
            var bytes: [UInt8] = []
            while value > 0 {
                bytes.insert(UInt8(value & 0xff), at: 0)
                value >>= 8
            }
            out.append(UInt8(0x80 | bytes.count))
            out.append(contentsOf: bytes)
        }
        out.append(body)
        return out
    }

    static func concat(_ parts: [Data]) -> Data {
        var data = Data()
        for part in parts { data.append(part) }
        return data
    }

    static func sequence(_ parts: Data...) -> Data { sequence(parts) }
    static func sequence(_ parts: [Data]) -> Data { tlv(0x30, concat(parts)) }
    static func set(_ parts: Data...) -> Data { tlv(0x31, concat(parts)) }

    static func integer(_ value: Int) -> Data {
        var bytes: [UInt8] = []
        var number = value
        repeat {
            bytes.insert(UInt8(number & 0xff), at: 0)
            number >>= 8
        } while number > 0
        if bytes[0] & 0x80 != 0 { bytes.insert(0, at: 0) }
        return tlv(0x02, Data(bytes))
    }

    static func integer(_ bytes: Data) -> Data {
        var value = bytes
        if let first = value.first, first & 0x80 != 0 { value.insert(0, at: 0) }
        return tlv(0x02, value)
    }

    static func oid(_ text: String) -> Data {
        let parts = text.split(separator: ".").compactMap { Int($0) }
        guard parts.count >= 2 else { return tlv(0x06, Data()) }
        var body = Data([UInt8(parts[0] * 40 + parts[1])])
        for part in parts.dropFirst(2) {
            var value = part
            var encoded: [UInt8] = [UInt8(value & 0x7f)]
            value >>= 7
            while value > 0 {
                encoded.insert(UInt8((value & 0x7f) | 0x80), at: 0)
                value >>= 7
            }
            body.append(contentsOf: encoded)
        }
        return tlv(0x06, body)
    }

    static func utf8(_ text: String) -> Data { tlv(0x0c, Data(text.utf8)) }
    static func boolean(_ value: Bool) -> Data { tlv(0x01, Data([value ? 0xff : 0x00])) }
    static func null() -> Data { tlv(0x05, Data()) }

    static func bitString(_ bytes: Data, unused: UInt8) -> Data {
        var body = Data([unused])
        body.append(bytes)
        return tlv(0x03, body)
    }

    static func utcTime(_ date: Date) -> Data {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let year = calendar.component(.year, from: date)
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "UTC")
        if year >= 1950, year < 2050 {
            formatter.dateFormat = "yyMMddHHmmss'Z'"
            return tlv(0x17, Data(formatter.string(from: date).utf8))
        }
        formatter.dateFormat = "yyyyMMddHHmmss'Z'"
        return tlv(0x18, Data(formatter.string(from: date).utf8))
    }

    static func explicit(_ tag: UInt8, _ body: Data) -> Data { tlv(tag, body) }
    static func contextPrimitive(_ tag: UInt8, _ body: Data) -> Data { tlv(0x80 | tag, body) }
}

struct DERNode {
    let tag: UInt8
    let value: Data
    let raw: Data

    var children: [DERNode] {
        (try? DERNode.parseChildren(value)) ?? []
    }

    static func parseSingle(_ data: Data) throws -> DERNode {
        let bytes = [UInt8](data)
        var offset = 0
        return try parse(bytes, offset: &offset)
    }

    static func parseChildren(_ data: Data) throws -> [DERNode] {
        let bytes = [UInt8](data)
        var offset = 0
        var result: [DERNode] = []
        while offset < bytes.count {
            result.append(try parse(bytes, offset: &offset))
        }
        return result
    }

    private static func parse(_ bytes: [UInt8], offset: inout Int) throws -> DERNode {
        let start = offset
        guard offset + 2 <= bytes.count else { throw DERError.malformed }
        let tag = bytes[offset]
        offset += 1
        var length = Int(bytes[offset])
        offset += 1
        if length & 0x80 != 0 {
            let count = length & 0x7f
            guard count > 0, count <= 4, offset + count <= bytes.count else { throw DERError.malformed }
            length = 0
            for _ in 0..<count {
                length = (length << 8) | Int(bytes[offset])
                offset += 1
            }
        }
        guard length >= 0, offset + length <= bytes.count else { throw DERError.malformed }
        let value = Data(bytes[offset..<(offset + length)])
        offset += length
        return DERNode(tag: tag, value: value, raw: Data(bytes[start..<offset]))
    }
}

enum DERError: Error { case malformed }

struct CertificateInfo {
    let subject: Data
    let issuer: Data
    let notBefore: Date
    let notAfter: Date
    let dnsNames: [String]
}

enum X509Parser {
    static func info(_ der: Data) throws -> CertificateInfo {
        let root = try DERNode.parseSingle(der)
        guard root.tag == 0x30, let tbs = root.children.first else { throw DERError.malformed }
        var fields = tbs.children
        guard !fields.isEmpty else { throw DERError.malformed }
        if fields[0].tag == 0xa0 { fields.removeFirst() }
        guard fields.count >= 6 else { throw DERError.malformed }
        let issuer = fields[2]
        let validity = fields[3]
        let subject = fields[4]
        let times = validity.children
        guard times.count >= 2 else { throw DERError.malformed }
        let notBefore = parseTime(times[0])
        let notAfter = parseTime(times[1])

        var dnsNames: [String] = []
        for field in fields.dropFirst(6) where field.tag == 0xa3 {
            let extensions = field.children.first?.children ?? []
            for ext in extensions {
                let parts = ext.children
                guard parts.count >= 3, parts[0].tag == 0x06,
                      parts[0].value == Data([0x55, 0x1d, 0x11]) else { continue }
                guard let octet = parts.last, octet.tag == 0x04,
                      let sequence = try? DERNode.parseSingle(octet.value) else { continue }
                for name in sequence.children where name.tag == 0x82 {
                    if let text = String(data: name.value, encoding: .ascii) { dnsNames.append(text) }
                }
            }
        }
        return CertificateInfo(
            subject: subject.raw, issuer: issuer.raw,
            notBefore: notBefore, notAfter: notAfter, dnsNames: dnsNames
        )
    }

    private static func parseTime(_ node: DERNode) -> Date {
        let format: String
        if node.tag == 0x17 {
            format = "yyMMddHHmmss'Z'"
        } else if node.tag == 0x18 {
            format = "yyyyMMddHHmmss'Z'"
        } else {
            return Date()
        }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.dateFormat = format
        guard let text = String(data: node.value, encoding: .ascii),
              let date = formatter.date(from: text) else { return Date() }
        return date
    }
}
