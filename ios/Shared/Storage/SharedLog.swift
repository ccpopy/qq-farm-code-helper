import Foundation

struct LogRecord: Codable, Equatable, Sendable {
    var time: Date
    var text: String
}

enum LogSource: String, Codable, Sendable {
    case app
    case tunnel
}

/// Only bounded, redacted diagnostics live here. No Code, Token, GID or URL is retained.
enum SharedLog {
    private static let lock = NSLock()
    private static let maxRecords = 200
    private static let appKey = "logs.app.v1"
    private static let tunnelKey = "logs.tunnel.v1"
    private static let formatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "HH:mm:ss"
        return formatter
    }()

    static func append(_ tag: String, _ text: String, source: LogSource) {
        let safe = sanitize(text)
        let record = LogRecord(time: Date(), text: formatter.string(from: Date()) + "  " + tag + "  " + safe)
        lock.lock()
        var records = load(source)
        records.append(record)
        if records.count > maxRecords { records.removeFirst(records.count - maxRecords) }
        save(records, source: source)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    static func text() -> String {
        lock.lock()
        let records = (load(.app) + load(.tunnel)).sorted { $0.time < $1.time }
        lock.unlock()
        return records.map(\.text).joined(separator: "\n")
    }

    static func clear() {
        lock.lock()
        save([], source: .app)
        save([], source: .tunnel)
        lock.unlock()
        AppGroup.notifyCaptureChanged()
    }

    private static func key(_ source: LogSource) -> String {
        source == .app ? appKey : tunnelKey
    }

    private static func load(_ source: LogSource) -> [LogRecord] {
        guard let data = AppGroup.defaults.data(forKey: key(source)) else { return [] }
        return (try? JSONDecoder().decode([LogRecord].self, from: data)) ?? []
    }

    private static func save(_ records: [LogRecord], source: LogSource) {
        guard let data = try? JSONEncoder().encode(records) else { return }
        AppGroup.defaults.set(data, forKey: key(source))
    }

    private static func sanitize(_ text: String) -> String {
        var value = text
        value = value.replacingOccurrences(
            of: "(?i)((?:code|token|secret|authorization|cookie|open_?id|gid)\\s*[=:]\\s*)[^\\s&,;]+",
            with: "$1[已隐藏]",
            options: .regularExpression
        )
        value = value.replacingOccurrences(
            of: "https?://[^\\s]+",
            with: "[地址已隐藏]",
            options: .regularExpression
        )
        if value.count > 400 { value = String(value.prefix(400)) }
        return value
    }
}
