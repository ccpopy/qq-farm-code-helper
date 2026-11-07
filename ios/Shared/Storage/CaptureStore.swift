import CryptoKit
import Foundation

/// Cross-process capture store. Every read re-reads the App Group defaults, so clearing a result
/// in the app immediately invalidates the tunnel extension's in-flight generation.
final class CaptureStore: @unchecked Sendable {
    static let shared = CaptureStore()

    private let defaults = AppGroup.defaults
    private let sessionKey = "capture.session.v1"
    private let generationKey = "capture.generation.v1"
    private let lock = NSLock()
    private var keyCache: Data?
    private let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }()
    private let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()

    // MARK: - Primitive access

    func generation() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return defaults.integer(forKey: generationKey)
    }

    private func nextGenerationUnlocked() -> Int {
        let next = max(1, defaults.integer(forKey: generationKey) + 1)
        defaults.set(next, forKey: generationKey)
        return next
    }

    private func readUnlocked() -> CaptureSession? {
        guard let raw = defaults.data(forKey: sessionKey),
              let data = open(raw),
              var session = try? decoder.decode(CaptureSession.self, from: data) else { return nil }
        if !session.code.isEmpty, Date().timeIntervalSince(session.createdAt) >= CaptureSession.codeLifetime {
            session.code = ""
            session.expiredCode = true
            session.updatedAt = Date()
            writeUnlocked(session)
        }
        return session
    }

    private func writeUnlocked(_ session: CaptureSession?) {
        if let session, let data = try? encoder.encode(session), let sealed = seal(data) {
            defaults.set(sealed, forKey: sessionKey)
        } else {
            defaults.removeObject(forKey: sessionKey)
        }
    }

    private func keyData() -> Data? {
        if let keyCache { return keyCache }
        let value = KeychainStore.captureKeyData()
        keyCache = value
        return value
    }

    private func seal(_ data: Data) -> Data? {
        guard let keyData = keyData() else { return nil }
        let key = SymmetricKey(data: keyData)
        return try? AES.GCM.seal(data, using: key).combined
    }

    private func open(_ data: Data) -> Data? {
        guard let keyData = keyData() else { return data }
        let key = SymmetricKey(data: keyData)
        guard let box = try? AES.GCM.SealedBox(combined: data) else {
            // Backward compatibility: an unencrypted snapshot from an earlier build.
            return data
        }
        return try? AES.GCM.open(box, using: key)
    }

    // MARK: - Public state

    func current() -> CaptureSession? {
        lock.lock()
        defer { lock.unlock() }
        return readUnlocked()
    }

    func hasPending() -> Bool {
        guard let session = current() else { return false }
        return session.hasPending
    }

    /// A VPN start always begins a fresh round, matching the Android service.
    @discardableResult
    func beginRound(platform: CapturePlatform) -> Int {
        lock.lock()
        let generation = nextGenerationUnlocked()
        writeUnlocked(nil)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
        return generation
    }

    func clear() {
        lock.lock()
        _ = nextGenerationUnlocked()
        writeUnlocked(nil)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    /// Claims the current round for one TCP connection. Returns false if another connection
    /// already published a result or the round was cleared meanwhile.
    func publish(code: String, generation: Int, platform: CapturePlatform) -> Bool {
        lock.lock()
        guard generation == defaults.integer(forKey: generationKey), readUnlocked() == nil else {
            lock.unlock()
            return false
        }
        let now = Date()
        let session = CaptureSession(
            generation: generation,
            platform: platform,
            code: code,
            ownGid: "",
            gids: [],
            createdAt: now,
            friendsObserved: false,
            collecting: true,
            published: true,
            consumedGids: [],
            expiredCode: false,
            updatedAt: now
        )
        writeUnlocked(session)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
        return true
    }

    /// Merges identity / friend deltas. `complete` marks a full snapshot (GetAll/SyncAll).
    func merge(generation: Int, platform: CapturePlatform, ownGid: String, gids: [String], friendsObserved: Bool, complete: Bool) -> Bool {
        lock.lock()
        guard generation == defaults.integer(forKey: generationKey),
              var session = readUnlocked(),
              session.platform == platform else {
            lock.unlock()
            return false
        }
        if !session.ownGid.isEmpty, !ownGid.isEmpty, session.ownGid != ownGid {
            lock.unlock()
            return false
        }
        if !ownGid.isEmpty { session.ownGid = ownGid }

        var merged: [String]
        if complete {
            merged = gids
        } else {
            var seen = Set(session.gids)
            merged = session.gids
            for gid in gids where seen.insert(gid).inserted { merged.append(gid) }
        }
        let own = session.ownGid
        let consumed = Set(session.consumedGids)
        merged = merged.filter { $0 != own && !consumed.contains($0) }
        session.gids = merged
        session.friendsObserved = session.friendsObserved || friendsObserved
        session.collecting = !complete
        session.updatedAt = Date()
        writeUnlocked(session)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
        return true
    }

    func consumeCode(generation: Int) {
        lock.lock()
        if var session = readUnlocked(), session.generation == generation {
            session.code = ""
            session.updatedAt = Date()
            writeUnlocked(session)
        }
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    func consumeFriends(generation: Int, gids: [String]) {
        lock.lock()
        if var session = readUnlocked(), session.generation == generation {
            var consumed = Set(session.consumedGids)
            consumed.formUnion(gids)
            session.consumedGids = Array(consumed)
            let remove = Set(gids)
            session.gids.removeAll { remove.contains($0) }
            session.updatedAt = Date()
            writeUnlocked(session)
        }
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    func setManual(code: String, gids: [String], platform: CapturePlatform, ownGid: String = "") {
        lock.lock()
        let generation = nextGenerationUnlocked()
        let now = Date()
        let session = CaptureSession(
            generation: generation,
            platform: platform,
            code: code,
            ownGid: ownGid,
            gids: gids,
            createdAt: now,
            friendsObserved: false,
            collecting: false,
            published: true,
            consumedGids: [],
            expiredCode: false,
            updatedAt: now
        )
        writeUnlocked(session)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    func markTunnelStopped() {
        lock.lock()
        if var session = readUnlocked() {
            session.collecting = false
            session.updatedAt = Date()
            writeUnlocked(session)
        }
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    /// A round's relay has closed. Available results stay valid for pushing.
    func finish(generation: Int) {
        lock.lock()
        if var session = readUnlocked(), session.generation == generation {
            session.collecting = false
            session.updatedAt = Date()
            writeUnlocked(session)
        }
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    func statusText(capturing: Bool, platform: CapturePlatform) -> String {
        if let session = current() { return session.summary(capturing: capturing) }
        return capturing ? "等待进入 \(platform.label) 农场" : "暂无获取结果"
    }
}
