import CoreFoundation
import Foundation

enum AppGroup {
    static let identifier = "group.io.github.ccpopy.farmhelper"
    static let captureChanged = "io.github.ccpopy.farmhelper.captureChanged"
    static let defaults: UserDefaults = UserDefaults(suiteName: identifier) ?? .standard

    static var containerURL: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier)
    }

    static var keychainAccessGroup: String? {
        Bundle.main.object(forInfoDictionaryKey: "SharedKeychainAccessGroup") as? String
    }

    static func notifyCaptureChanged() {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName(captureChanged as CFString),
            nil, nil, true
        )
    }
}

/// Small wrapper around the Darwin notification center, so both processes can wake each other
/// without polling. Callbacks are kept in a registry keyed by the opaque observer pointer.
final class DarwinNotifier: @unchecked Sendable {
    static let shared = DarwinNotifier()

    private struct Entry {
        let handler: () -> Void
    }

    private let lock = NSLock()
    private var entries: [String: [UUID: Entry]] = [:]
    private var installed: Set<String> = []
    private var observerPointer: UnsafeMutableRawPointer { Unmanaged.passUnretained(self).toOpaque() }

    @discardableResult
    func add(_ name: String, handler: @escaping () -> Void) -> UUID {
        let id = UUID()
        lock.lock()
        entries[name, default: [:]][id] = Entry(handler: handler)
        let needsInstall = !installed.contains(name)
        if needsInstall { installed.insert(name) }
        lock.unlock()
        if needsInstall { install(name) }
        return id
    }

    func remove(_ name: String, id: UUID) {
        lock.lock()
        entries[name]?.removeValue(forKey: id)
        lock.unlock()
    }

    fileprivate func fire(_ name: String) {
        lock.lock()
        let handlers = entries[name]?.values.map(\.handler) ?? []
        lock.unlock()
        handlers.forEach { $0() }
    }

    private func install(_ name: String) {
        let callback: CFNotificationCallback = { _, observer, noteName, _, _ in
            guard let observer, let noteName else { return }
            let note = noteName.rawValue as String
            Unmanaged<DarwinNotifier>.fromOpaque(observer).takeUnretainedValue().fire(note)
        }
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDarwinNotifyCenter(),
            observerPointer,
            callback,
            name as CFString,
            nil,
            .deliverImmediately
        )
    }
}
