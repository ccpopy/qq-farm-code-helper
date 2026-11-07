import Foundation

enum TunnelError: LocalizedError {
    case timeout
    case closed
    case tls(String)
    case invalid(String)
    case socket(String)

    var errorDescription: String? {
        switch self {
        case .timeout: return "连接超时"
        case .closed: return "连接已关闭"
        case .tls(let text): return "TLS 失败：\(text)"
        case .invalid(let text): return text
        case .socket(let text): return "本地转发失败：\(text)"
        }
    }
}
