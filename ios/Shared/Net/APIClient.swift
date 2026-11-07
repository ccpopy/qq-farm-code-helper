import Foundation

enum APIError: LocalizedError, Equatable {
    case auth
    case relayUnsupported
    case temporary
    case redirect
    case http(Int)
    case serverRejected
    case invalidResponse(String)
    case uncertainWxSave
    case message(String)

    var errorDescription: String? {
        switch self {
        case .auth: return "管理 Token 已失效或权限不足，请重新登录服务器并复制 Token"
        case .relayUnsupported: return "服务器尚未提供手机中转"
        case .temporary: return "网络连接暂时中断，请稍后重试"
        case .redirect: return "服务器返回跳转；为保护 Token，请直接填写最终服务器地址"
        case .http(let status): return "服务器请求失败（HTTP \(status)）"
        case .serverRejected: return "服务器拒绝操作，请到管理页面检查账号和权限"
        case .invalidResponse(let text): return text
        case .uncertainWxSave: return "登录提交结果未能确认。请先到服务器账号页检查，勿重复新增；本次不会自动重试提交。"
        case .message(let text): return text
        }
    }
}

private final class NoRedirectDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        // An admin token must never follow a redirect to another host.
        completionHandler(nil)
    }
}

/// Admin API client. One instance is pinned to one server profile and one credential generation.
final class APIClient: @unchecked Sendable {
    let serverId: String
    let baseURL: String
    let connectionId: String
    private let token: String
    private let settings: SharedSettings

    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 60
        configuration.httpCookieStorage = nil
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.httpShouldSetCookies = false
        return URLSession(configuration: configuration, delegate: NoRedirectDelegate(), delegateQueue: nil)
    }()

    init(server: ServerProfile, settings: SharedSettings = .shared) throws {
        let base = try UrlPolicy.server(server.baseURL, allowHTTP: server.allowHTTP)
        let token = settings.token(for: server.id)
        guard !token.isEmpty else { throw APIError.message("请先保存服务器与 Token") }
        self.serverId = server.id
        self.baseURL = base
        self.token = token
        self.connectionId = server.connectionId
        self.settings = settings
    }

    var isCurrent: Bool {
        settings.server(id: serverId)?.connectionId == connectionId
    }

    var profile: ServerProfile? { settings.server(id: serverId) }

    // MARK: - Push gate

    private static let pushLock = NSLock()
    private static var pushRunning = false
    static var isPushing: Bool {
        pushLock.lock()
        defer { pushLock.unlock() }
        return pushRunning
    }

    private static func beginPush() throws {
        pushLock.lock()
        if pushRunning {
            pushLock.unlock()
            throw APIError.message("正在推送，请等待本次完成")
        }
        pushRunning = true
        pushLock.unlock()
    }

    private static func endPush() {
        pushLock.lock()
        pushRunning = false
        pushLock.unlock()
    }

    // MARK: - Accounts

    func accounts() async throws -> [Account] {
        try parseAccounts(try await call("GET", "/api/accounts"))
    }

    func knownGid(_ account: Account) -> String {
        account.gid.isEmpty ? settings.rememberedGid(serverId: serverId, connectionId: connectionId, account: account) : account.gid
    }

    func ping() async throws -> String {
        let response = try await call("GET", "/api/ping")
        let data = response["data"] as? [String: Any]
        let version = data?["version"] as? String ?? "未知版本"
        return data == nil ? "连接成功" : "连接成功 · 服务端 " + version
    }

    func connected(_ account: Account) async throws -> Bool {
        if !account.running { return false }
        let response = try await call("GET", "/api/status", accountId: account.id)
        guard let data = response["data"] as? [String: Any],
              let connection = data["connection"] as? [String: Any] else {
            throw APIError.invalidResponse("未返回账号连接状态")
        }
        return (connection["connected"] as? Bool) ?? false
    }

    // MARK: - Push / create / update

    func push(targetId: String, capture: CaptureSession, code: Bool, friends: Bool) async throws {
        guard code || friends else { throw APIError.message("没有待推送的数据") }
        try Self.beginPush()
        defer { Self.endPush() }
        try requirePending(capture, code: code, friends: friends)
        guard !targetId.isEmpty else { throw APIError.message("请先选择目标账号") }
        let list = try await accounts()
        guard let target = list.first(where: { $0.id == targetId }) else {
            throw APIError.message("目标账号已不存在，请重新选择")
        }
        guard target.platform == capture.platform.rawValue else {
            throw APIError.message("获取结果来自 \(capture.platform.label)，请选择同一平台的账号")
        }
        if !capture.ownGid.isEmpty, !target.gid.isEmpty, target.gid != capture.ownGid {
            throw APIError.message("捕获身份与服务器 GID 不一致，已阻止更新")
        }
        try requirePending(capture, code: code, friends: friends)
        if code {
            guard !capture.code.isEmpty else { throw APIError.message("没有可推送的 Code") }
            let body: [String: Any] = [
                "id": target.id, "name": target.name, "code": capture.code,
                "platform": capture.platform.rawValue, "loginType": "manual",
            ]
            _ = try await call("POST", "/api/accounts", body: body)
            CaptureStore.shared.consumeCode(generation: capture.generation)
            settings.setTarget(serverId: serverId, account: target)
            settings.rememberIdentity(serverId: serverId, connectionId: connectionId, account: target, capture: capture)
            SharedLog.append("推送", "Code 已更新；正在启动对应远程账号", source: .app)
            do {
                let targetId = try Self.safeId(target.id)
                _ = try await call("POST", "/api/accounts/\(targetId)/start", body: [:])
            } catch {
                throw APIError.message("Code 已更新，但启动失败；请在管理页面检查，不要重复使用旧 Code")
            }
        }
        if friends {
            try await appendFriends(targetId: target.id, capture: capture)
        }
    }

    func checkNewAccount(baseName: String, actualName: String, capture: CaptureSession, friends: Bool) async throws -> [Account] {
        try AccountNamePolicy.validate(baseName, actualName)
        guard !capture.code.isEmpty else {
            throw APIError.message("新增账号必须提供新的 Code；仅有好友 GID 时请选择已有账号")
        }
        if friends, capture.gids.isEmpty { throw APIError.message("没有待同步的好友 GID") }
        let existing = try await accounts()
        for account in existing {
            let name = account.name.trimmingCharacters(in: .whitespacesAndNewlines)
            if name == baseName.trimmingCharacters(in: .whitespacesAndNewlines) || name == actualName {
                throw APIError.message("服务器已有同名备注；上游同名会更新旧账号，请改用不同备注或选择更新已有账号")
            }
            if capture.platform.rawValue == account.platform, !capture.ownGid.isEmpty,
               capture.ownGid == knownGid(account) {
                throw APIError.message("这个 GID 已存在于服务器，请选择对应账号更新，不要重复新增")
            }
        }
        return existing
    }

    func create(baseName: String, actualName: String, capture: CaptureSession, friends: Bool) async throws -> Account {
        try Self.beginPush()
        defer { Self.endPush() }
        try requirePending(capture, code: true, friends: friends)
        let before = try await checkNewAccount(baseName: baseName, actualName: actualName, capture: capture, friends: friends)
        try requirePending(capture, code: true, friends: friends)
        let body: [String: Any] = [
            "name": actualName, "code": capture.code,
            "platform": capture.platform.rawValue, "loginType": "manual",
        ]
        let response: [String: Any]
        do {
            response = try await call("POST", "/api/accounts", body: body)
        } catch let error as APIError where error == .auth {
            throw error
        } catch {
            CaptureStore.shared.consumeCode(generation: capture.generation)
            throw APIError.message("未能确认新增结果；为避免重复新增，Code 已清除。请先检查服务器账号列表，再重新获取")
        }
        CaptureStore.shared.consumeCode(generation: capture.generation)
        let previous = Set(before.map(\.id))
        let created = try parseAccounts(response).filter {
            !previous.contains($0.id) && $0.name == actualName && $0.platform == capture.platform.rawValue
        }
        guard created.count == 1, let account = created.first else {
            throw APIError.message("新增请求已提交，但无法确认服务器分配的账号 ID；Code 已清除，请到账号页刷新选择，不要重复新增")
        }
        guard isCurrent else {
            throw APIError.message("新增请求已提交，但服务器配置已变化；不会绑定到新配置，请在原服务器确认")
        }
        settings.setTarget(serverId: serverId, account: account)
        settings.rememberIdentity(serverId: serverId, connectionId: connectionId, account: account, capture: capture)
        SharedLog.append("推送", "新账号已创建并绑定服务器返回的 ID；由服务器按其规则自动启动", source: .app)
        if friends {
            do {
                try await appendFriends(targetId: account.id, capture: capture)
            } catch {
                throw APIError.message("新账号已创建并绑定，但好友同步未完成；可点击「只同步好友」重试，不要重复新增")
            }
        }
        return account
    }

    private func appendFriends(targetId: String, capture: CaptureSession) async throws {
        try requirePending(capture, code: false, friends: true)
        let gids = try UrlPolicy.gids(capture.gids.joined(separator: ","), ownGid: capture.ownGid)
        guard !gids.isEmpty else { throw APIError.message("没有好友 GID 可同步；Code 若已成功提交则不会再次使用") }
        _ = try await call("POST", "/api/friend-known-gids/batch-add", body: ["gids": gids], accountId: targetId)
        CaptureStore.shared.consumeFriends(generation: capture.generation, gids: gids)
        SharedLog.append("推送", "好友已增量同步，共 \(gids.count) 个；未覆盖原有好友", source: .app)
    }

    private func requirePending(_ capture: CaptureSession, code: Bool, friends: Bool) throws {
        guard isCurrent, let current = CaptureStore.shared.current(),
              current.generation == capture.generation,
              current.platform == capture.platform else {
            throw APIError.message("服务器或本次结果已变化，请重新确认")
        }
        if code, (capture.code.isEmpty || capture.code != current.code) {
            throw APIError.message("Code 已推送或过期，请重新获取")
        }
        if friends, (capture.gids.isEmpty || !Set(current.gids).isSuperset(of: Set(capture.gids))) {
            throw APIError.message("好友结果已变化或已同步，请重新确认")
        }
    }

    // MARK: - WeChat QR login

    struct WxTask: Sendable, Equatable {
        let id: String
        let expiresAt: Date
    }

    struct WxDraft: Sendable, Equatable {
        let id: String
        let baseName: String
        let name: String
        let previousIds: Set<String>
    }

    func prepareWxAccount(targetId: String, baseName: String, actualName: String) async throws -> WxDraft {
        let list = try await accounts()
        if !targetId.isEmpty {
            guard let account = list.first(where: { $0.id == targetId }) else {
                throw APIError.message("目标账号已不存在，请重新选择")
            }
            guard account.platform == "wx" else { throw APIError.message("只能更新微信账号") }
            return WxDraft(id: account.id, baseName: "", name: account.name, previousIds: [])
        }
        try AccountNamePolicy.validate(baseName, actualName)
        var ids = Set<String>()
        for account in list {
            let name = account.name.trimmingCharacters(in: .whitespacesAndNewlines)
            if name == baseName.trimmingCharacters(in: .whitespacesAndNewlines) || name == actualName {
                throw APIError.message("已有同名账号，请选择更新已有账号或更换备注")
            }
            ids.insert(account.id)
        }
        return WxDraft(id: "", baseName: baseName, name: actualName, previousIds: ids)
    }

    func createWxTask() async throws -> WxTask {
        let response = try await call("POST", "/api/wx-login/tasks", body: ["app_id": "wx5306c5978fdb76e4"])
        guard let data = response["data"] as? [String: Any],
              let taskId = data["task_id"] as? String,
              let expires = data["expires_at"] as? NSNumber else {
            throw APIError.invalidResponse("服务器未返回扫码任务")
        }
        return WxTask(id: try Self.safeId(taskId), expiresAt: Date(timeIntervalSince1970: expires.doubleValue))
    }

    func wxQr(_ task: WxTask) async throws -> Data {
        try await request("GET", wxPath(task) + "/qr", body: nil, accountId: nil, readTimeout: 15, expectImage: true)
    }

    func wxStatus(_ task: WxTask) async throws -> String {
        let response = try await call("GET", wxPath(task) + "/status", readTimeout: 40)
        guard let data = response["data"] as? [String: Any], let status = data["status"] as? String else {
            throw APIError.invalidResponse("服务器未返回扫码状态")
        }
        return status
    }

    func cancelWxTask(_ task: WxTask) async throws {
        _ = try await call("DELETE", wxPath(task))
    }

    func finishWxLogin(task: WxTask, draft: WxDraft) async throws -> Account {
        let checked = try await prepareWxAccount(targetId: draft.id, baseName: draft.baseName, actualName: draft.name)
        guard checked.name == draft.name else { throw APIError.message("目标账号备注已变化，请重新确认") }
        _ = try await call("POST", wxPath(task) + "/confirm", readTimeout: 40)
        let codeResponse = try await call("POST", wxPath(task) + "/code", readTimeout: 40)
        guard let codeData = codeResponse["data"] as? [String: Any],
              let code = codeData["code"] as? String, !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw APIError.message("服务器未取得农场 Code，请重新扫码")
        }
        var body: [String: Any] = [
            "name": draft.name, "code": code, "platform": "wx",
            "loginType": "wechat_qr", "wxLoginTaskId": task.id,
        ]
        if !draft.id.isEmpty { body["id"] = draft.id }
        let response: [String: Any]
        do {
            response = try await call("POST", "/api/accounts", body: body)
        } catch let error as APIError where error == .auth {
            throw error
        } catch {
            throw APIError.uncertainWxSave
        }
        let candidates = try parseAccounts(response).filter {
            let identity = draft.id.isEmpty ? !checked.previousIds.contains($0.id) : $0.id == draft.id
            return identity && $0.platform == "wx" && $0.name == draft.name
        }
        guard candidates.count == 1, let saved = candidates.first, isCurrent else {
            throw APIError.uncertainWxSave
        }
        settings.setTarget(serverId: serverId, account: saved)
        SharedLog.append("微信扫码", "农场 Code 与扫码会话已提交服务器；正在检查登录与好友", source: .app)
        return saved
    }

    func wechatFriendCount(targetId: String) async throws -> Int {
        let response = try await call("GET", "/api/friends?forceSync=true", accountId: targetId, readTimeout: 40)
        guard let data = response["data"] as? [[String: Any]] else {
            throw APIError.invalidResponse("服务器未返回好友列表")
        }
        var gids = Set<String>()
        for item in data {
            let gid = item["gid"] as? String ?? ""
            guard gid.range(of: "^[1-9][0-9]*$", options: .regularExpression) != nil else {
                throw APIError.message("好友响应缺少有效 GID，请在服务器确认")
            }
            gids.insert(gid)
        }
        return gids.count
    }

    private func wxPath(_ task: WxTask) -> String { "/api/wx-login/tasks/" + ((try? Self.safeId(task.id)) ?? "") }

    // MARK: - Offline events

    struct OfflineEvent: Sendable, Equatable {
        let eventId: String
        let accountId: String
        let accountUuid: String
        let platform: String
    }

    struct OfflineBatch: Sendable, Equatable {
        let cursor: String
        let events: [OfflineEvent]
    }

    func offlineEvents(cursor: String, selected: Set<String>) async throws -> OfflineBatch {
        guard cursor.range(of: "^[A-Za-z0-9_:-]{0,100}$", options: .regularExpression) != nil else {
            throw APIError.message("通知读取位置无效")
        }
        guard !selected.isEmpty else { throw APIError.message("请先选择需要提醒的账号") }
        let ids = try selected.map { try Self.safeId($0) }.joined(separator: ",")
        let path = "/api/mobile/offline-events?after=\(cursor)&wait=25&accounts=\(ids)"
        let response = try await call("GET", path, readTimeout: 35)
        guard let data = response["data"] as? [String: Any],
              let next = data["cursor"] as? String,
              next.range(of: "^[A-Za-z0-9_-]{1,64}:[0-9]{1,20}$", options: .regularExpression) != nil,
              let array = data["events"] as? [[String: Any]] else {
            throw APIError.invalidResponse("服务器通知格式不正确")
        }
        var events: [OfflineEvent] = []
        for event in array {
            guard let type = event["type"] as? String, type == "relogin_required",
                  let accountId = event["accountId"] as? String,
                  let uuid = event["accountUuid"] as? String, !uuid.isEmpty,
                  let eventId = event["eventId"] as? String,
                  let platform = event["platform"] as? String,
                  selected.contains(accountId) else {
                throw APIError.invalidResponse("服务器通知格式不正确")
            }
            events.append(OfflineEvent(
                eventId: eventId, accountId: try Self.safeId(accountId),
                accountUuid: uuid, platform: platform
            ))
        }
        return OfflineBatch(cursor: next, events: events)
    }

    // MARK: - Transport

    private func call(
        _ method: String,
        _ path: String,
        body: [String: Any]? = nil,
        accountId: String? = nil,
        readTimeout: TimeInterval = 15
    ) async throws -> [String: Any] {
        let data = try await request(method, path, body: body, accountId: accountId, readTimeout: readTimeout, expectImage: false)
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw APIError.invalidResponse("服务器响应格式不正确")
        }
        if (json["ok"] as? Bool) == false { throw APIError.serverRejected }
        return json
    }

    private func request(
        _ method: String,
        _ path: String,
        body: [String: Any]?,
        accountId: String?,
        readTimeout: TimeInterval,
        expectImage: Bool
    ) async throws -> Data {
        guard isCurrent else { throw APIError.message("服务器配置已变更，请重新选择账号后重试") }
        guard let url = URL(string: baseURL + path) else {
            throw APIError.message("服务器地址格式不正确")
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = readTimeout
        request.setValue(token, forHTTPHeaderField: "x-admin-token")
        request.setValue(expectImage ? "image/*" : "application/json", forHTTPHeaderField: "Accept")
        if let accountId {
            request.setValue(try Self.safeId(accountId), forHTTPHeaderField: "x-account-id")
        }
        if let body {
            request.setValue("application/json; charset=utf-8", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await Self.session.data(for: request)
        } catch {
            throw Self.mapTransportError(error)
        }
        guard let http = response as? HTTPURLResponse else {
            throw APIError.invalidResponse("服务器未返回可识别响应")
        }
        switch http.statusCode {
        case 200:
            break
        case 401, 403:
            throw APIError.auth
        case 404 where path.hasPrefix("/api/mobile/offline-events?"):
            throw APIError.relayUnsupported
        case 404 where path.hasPrefix("/api/wx-login/"):
            throw APIError.message("扫码任务已失效，或服务器尚未支持微信扫码登录，请重新获取或更新服务器")
        case 300..<400:
            throw APIError.redirect
        case 408, 429, 502, 503, 504:
            throw APIError.temporary
        default:
            throw APIError.http(http.statusCode)
        }
        guard data.count <= 4 * 1024 * 1024 else {
            throw APIError.invalidResponse("响应过大，已停止读取")
        }
        guard isCurrent else { throw APIError.message("服务器配置已变更，请重新开始") }
        return data
    }

    private static func mapTransportError(_ error: Error) -> Error {
        guard let urlError = error as? URLError else { return APIError.message("网络请求失败，请稍后重试") }
        switch urlError.code {
        case .timedOut, .networkConnectionLost, .notConnectedToInternet, .cannotConnectToHost,
             .cannotFindHost, .dnsLookupFailed, .resourceUnavailable:
            return APIError.temporary
        case .secureConnectionFailed, .serverCertificateHasBadDate, .serverCertificateUntrusted,
             .serverCertificateHasUnknownRoot, .serverCertificateNotYetValid, .clientCertificateRejected:
            return APIError.message("服务器 HTTPS 证书验证失败；不会忽略 TLS 错误")
        default:
            return APIError.message("网络请求失败，请检查服务器地址与网络")
        }
    }

    private static func safeId(_ value: String) throws -> String {
        guard value.range(of: "^[A-Za-z0-9_-]{1,128}$", options: .regularExpression) != nil else {
            throw APIError.message("账号 ID 格式不正确")
        }
        return value
    }

    private func parseAccounts(_ response: [String: Any]) throws -> [Account] {
        let data = response["data"]
        let array: [[String: Any]]?
        if let object = data as? [String: Any] {
            array = object["accounts"] as? [[String: Any]]
        } else {
            array = data as? [[String: Any]]
        }
        guard let array else { throw APIError.invalidResponse("服务器未返回可识别的账号列表") }
        return array.compactMap { Account(json: $0) }
    }
}
