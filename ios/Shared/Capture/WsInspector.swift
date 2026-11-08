import Foundation

/// Passive RFC 6455 / protobuf reader. Never retains names, avatars, OpenIDs or full frames.
/// Only GameFriend.gid (field 1, varint) and BasicInfo.gid are accepted as identifiers.
final class WsInspector: @unchecked Sendable {
    private static let maxSize = 8 * 1024 * 1024
    private var buffer = [UInt8]()
    private var start = 0
    private var end = 0
    private var valid = true
    private var fragmented: [UInt8]?
    private var fragmentType = 0

    private(set) var ownGid = ""
    private(set) var friendsObserved = false
    private(set) var friendsComplete = false
    private(set) var revision: Int = 0
    private var gids: [String] = []

    var isValid: Bool { valid }
    var pendingGids: [String] { gids.filter { $0 != ownGid } }

    func feed(_ data: [UInt8], offset: Int = 0, length: Int? = nil) {
        let count = length ?? (data.count - offset)
        guard valid, count > 0 else { return }
        if count > Self.maxSize + 14 - (end - start) {
            disable()
            return
        }
        if buffer.count - end < count {
            let size = end - start
            let capacity = min(Self.maxSize + 14, max(size + count, buffer.count * 2))
            var next = [UInt8](repeating: 0, count: capacity)
            if size > 0 { next.replaceSubrange(0..<size, with: buffer[start..<end]) }
            buffer = next
            start = 0
            end = size
        }
        buffer.replaceSubrange(end..<(end + count), with: data[offset..<(offset + count)])
        end += count

        while end - start >= 2 {
            let first = Int(buffer[start])
            let second = Int(buffer[start + 1])
            let opcode = first & 15
            let fin = (first & 128) != 0
            guard (first & 112) == 0, (second & 128) == 0,
                  [0, 1, 2, 8, 9, 10].contains(opcode) else {
                disable()
                return
            }
            var header = 2
            var payload = second & 127
            if payload == 126 {
                guard end - start >= 4 else { return }
                payload = Int(buffer[start + 2]) << 8 | Int(buffer[start + 3])
                header = 4
                if payload < 126 { disable(); return }
            } else if payload == 127 {
                guard end - start >= 10 else { return }
                var value = 0
                for index in 2..<10 {
                    if value > Self.maxSize { disable(); return }
                    value = value * 256 + Int(buffer[start + index])
                }
                payload = value
                header = 10
                if payload < 65536 { disable(); return }
            }
            if payload > Self.maxSize || (opcode >= 8 && (!fin || payload > 125)) {
                disable()
                return
            }
            guard end - start >= header + payload else { return }
            let position = start + header
            if opcode <= 2 {
                if opcode != 0 {
                    guard fragmentType == 0 else { disable(); return }
                    if fin {
                        if opcode == 2 { inspect(Array(buffer[position..<(position + payload)])) }
                    } else {
                        fragmentType = opcode
                        fragmented = opcode == 2 ? Array(buffer[position..<(position + payload)]) : []
                    }
                } else {
                    guard fragmentType != 0 else { disable(); return }
                    if fragmentType == 2 {
                        if (fragmented?.count ?? 0) + payload > Self.maxSize { disable(); return }
                        fragmented?.append(contentsOf: buffer[position..<(position + payload)])
                    }
                    if fin {
                        if fragmentType == 2, let message = fragmented { inspect(message) }
                        fragmented = nil
                        fragmentType = 0
                    }
                }
            }
            start += header + payload
            if start == end {
                start = 0
                end = 0
                buffer = []
                break
            }
        }
    }

    private func disable() {
        valid = false
        buffer = []
        start = 0
        end = 0
        fragmented = nil
    }

    private func inspect(_ message: [UInt8]) {
        do {
            let envelope = Proto(message)
            var meta: [UInt8]?
            var body: [UInt8]?
            while try envelope.next() {
                if envelope.field == 1, envelope.wire == 2 { meta = try envelope.bytes() }
                else if envelope.field == 2, envelope.wire == 2 { body = try envelope.bytes() }
                else { try envelope.skip() }
            }
            guard let meta, let body else { return }

            let metaReader = Proto(meta)
            var service = ""
            var method = ""
            var type: UInt64 = 0
            var error: UInt64 = 0
            while try metaReader.next() {
                if metaReader.field == 1, metaReader.wire == 2 {
                    service = String(decoding: try metaReader.bytes(), as: UTF8.self)
                } else if metaReader.field == 2, metaReader.wire == 2 {
                    method = String(decoding: try metaReader.bytes(), as: UTF8.self)
                } else if metaReader.field == 3, metaReader.wire == 0 {
                    type = try metaReader.varint()
                } else if metaReader.field == 6, metaReader.wire == 0 {
                    error = try metaReader.varint()
                } else {
                    try metaReader.skip()
                }
            }
            guard type == 2, error == 0 else { return }

            if service == "gamepb.userpb.UserService", method == "Login" || method == "PlayerLogin" {
                let login = Proto(body)
                while try login.next() {
                    if login.field == 1, login.wire == 2 {
                        let entry = try login.bytes()
                        if let gid = try Self.firstGid(entry) {
                            ownGid = gid
                            revision += 1
                        }
                    } else {
                        try login.skip()
                    }
                }
            } else if service == "gamepb.friendpb.FriendService",
                      ["GetAll", "SyncAll", "GetGameFriends"].contains(method) {
                let friends = Proto(body)
                var found: [String] = []
                var seen = Set<String>()
                while try friends.next() {
                    if friends.field == 1, friends.wire == 2 {
                        let entry = try friends.bytes()
                        if let gid = try Self.firstGid(entry), seen.insert(gid).inserted {
                            found.append(gid)
                        }
                    } else {
                        try friends.skip()
                    }
                }
                let complete = method == "GetAll" || method == "SyncAll"
                if complete {
                    gids = found
                    friendsComplete = true
                } else {
                    var seenAll = Set(gids)
                    for gid in found where seenAll.insert(gid).inserted { gids.append(gid) }
                }
                friendsObserved = true
                revision += 1
            }
        } catch {
            // Malformed protobuf is not a successful capture.
        }
    }

    private static func firstGid(_ data: [UInt8]) throws -> String? {
        let entry = Proto(data)
        var gid: String?
        while try entry.next() {
            if entry.field == 1, entry.wire == 0 {
                let value = try entry.varint()
                if value > 0, value <= 9_007_199_254_740_991 {
                    gid = String(value)
                }
            } else {
                try entry.skip()
            }
        }
        return gid
    }

    private final class Proto {
        private let data: [UInt8]
        private var position = 0
        private(set) var field = 0
        private(set) var wire = 0

        init(_ data: [UInt8]) { self.data = data }

        func next() throws -> Bool {
            if position == data.count { return false }
            let tag = try varint()
            guard tag <= 0xffff_ffff, tag >> 3 > 0 else { throw ProtoError.malformed }
            field = Int(tag >> 3)
            wire = Int(tag & 7)
            return true
        }

        func varint() throws -> UInt64 {
            var value: UInt64 = 0
            for index in 0..<10 {
                guard position < data.count else { throw ProtoError.malformed }
                let byte = UInt64(data[position])
                position += 1
                if index == 9, byte > 1 { throw ProtoError.malformed }
                value |= (byte & 0x7f) << (index * 7)
                if byte & 0x80 == 0 { return value }
            }
            throw ProtoError.malformed
        }

        func bytes() throws -> [UInt8] {
            let size = try varint()
            guard size <= UInt64(data.count - position) else { throw ProtoError.malformed }
            let result = Array(data[position..<(position + Int(size))])
            position += Int(size)
            return result
        }

        func skip() throws {
            switch wire {
            case 0: _ = try varint()
            case 1:
                guard position + 8 <= data.count else { throw ProtoError.malformed }
                position += 8
            case 2:
                let size = try varint()
                guard size <= UInt64(data.count - position) else { throw ProtoError.malformed }
                position += Int(size)
            case 5:
                guard position + 4 <= data.count else { throw ProtoError.malformed }
                position += 4
            default: throw ProtoError.malformed
            }
        }
    }

    private enum ProtoError: Error { case malformed }
}
