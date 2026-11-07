import Foundation

struct UpdateInfo: Identifiable, Equatable {
    var id: String { version }
    let version: String
    let notes: String
    let pageURL: URL
}

/// iOS apps cannot install an update themselves. This only reports a tagged GitHub release and
/// opens its download page; installation stays with the user's signing/distribution method.
enum UpdateChecker {
    private static let api = "https://api.github.com/repos/ccpopy/qq-farm-code-helper/releases?per_page=30"
    private static let maximumResponse = 2 * 1024 * 1024

    static func check(settings: AppSettings) async throws -> UpdateInfo? {
        var data: Data
        do {
            data = try await fetch(api)
        } catch {
            guard settings.accelerated else { throw error }
            let base = try UrlPolicy.server(settings.mirror, allowHTTP: false)
            guard let mirrored = URL(string: base + "/" + api) else { throw SettingsError.message("加速地址无效") }
            data = try await fetch(mirrored.absoluteString)
        }
        guard let releases = try JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
            throw SettingsError.message("更新源响应格式不正确")
        }
        let current = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.0.0"
        var best: UpdateInfo?
        for release in releases {
            guard let tag = release["tag_name"] as? String,
                  tag.range(of: "^ios-v[0-9]+\\.[0-9]+\\.[0-9]+$", options: .regularExpression) != nil,
                  (release["prerelease"] as? Bool) != true,
                  (release["draft"] as? Bool) != true else { continue }
            let version = String(tag.dropFirst("ios-".count))
            guard compare(version, current) > 0 else { continue }
            guard let page = URL(string: "https://github.com/ccpopy/qq-farm-code-helper/releases/tag/\(tag)") else { continue }
            let notes = release["body"] as? String ?? ""
            if best == nil || compare(version, best!.version) > 0 {
                best = UpdateInfo(version: version, notes: notes, pageURL: page)
            }
        }
        return best
    }

    private static func fetch(_ address: String) async throws -> Data {
        guard let url = URL(string: address), url.scheme == "https" else {
            throw SettingsError.message("更新源必须是 HTTPS 地址")
        }
        var request = URLRequest(url: url)
        request.timeoutInterval = 20
        request.setValue("FarmHelperIOS/\(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0")", forHTTPHeaderField: "User-Agent")
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw SettingsError.message("更新源请求失败")
        }
        guard data.count <= maximumResponse else {
            throw SettingsError.message("更新源响应过大")
        }
        return data
    }

    static func compare(_ left: String, _ right: String) -> Int {
        let a = left.split(separator: ".").compactMap { Int($0) }
        let b = right.split(separator: ".").compactMap { Int($0) }
        guard a.count == 3, b.count == 3 else { return 0 }
        for index in 0..<3 {
            if a[index] != b[index] { return a[index] > b[index] ? 1 : -1 }
        }
        return 0
    }
}
