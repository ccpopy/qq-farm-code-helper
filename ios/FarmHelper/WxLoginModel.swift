import Combine
import Foundation
import UIKit

/// The short-lived QR task survives the trip to WeChat, but is never persisted.
@MainActor
final class WxLoginModel: ObservableObject {
    enum Phase: Equatable {
        case idle, preparing, waiting, scanned, authorized, finishing, saved, failed, uncertain
    }

    @Published var phase: Phase = .idle
    @Published var status = "准备就绪"
    @Published var detail = ""
    @Published var image: UIImage?
    @Published var servers: [ServerProfile] = []
    @Published var selectedServerId = ""
    @Published var targets: [Account] = []
    @Published var selectedTargetId = ""
    @Published var baseName = ""
    @Published var checking = false

    private let settings = SharedSettings.shared
    private let nonce = String(UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased().prefix(12))
    private var client: APIClient?
    private var task: APIClient.WxTask?
    private var draft: APIClient.WxDraft?
    private var saved: Account?
    private var pollTask: Task<Void, Never>?
    private var generation = 0
    private var pollFailures = 0
    private var profileConnection = ""

    var canStart: Bool { phase == .idle || phase == .failed }
    var canFinish: Bool { phase == .authorized }
    var hasLiveQR: Bool {
        guard let task else { return false }
        return image != nil && Date() < task.expiresAt
            && (phase == .waiting || phase == .scanned || phase == .authorized)
    }

    var taskEndText: String? {
        guard let task, Date() < task.expiresAt else { return nil }
        let remaining = max(0, Int(task.expiresAt.timeIntervalSinceNow))
        return String(format: "剩余 %d:%02d", remaining / 60, remaining % 60)
    }

    var server: ServerProfile? {
        settings.server(id: selectedServerId)
    }

    func load() {
        servers = settings.load().servers.filter { $0.ready(token: settings.token(for: $0.id)) }
        if !servers.contains(where: { $0.id == selectedServerId }) {
            selectedServerId = servers.first?.id ?? ""
        }
        pinServer()
        loadTargets()
    }

    func applyPreset(serverId: String, targetId: String) {
        if servers.contains(where: { $0.id == serverId }) {
            selectedServerId = serverId
            pinServer()
        }
        selectedTargetId = targetId
        loadTargets()
    }

    func selectServer(_ id: String) {
        guard phase == .idle || phase == .failed else { return }
        selectedServerId = id
        selectedTargetId = ""
        pinServer()
        loadTargets()
    }

    private func pinServer() {
        guard let server else {
            profileConnection = ""
            return
        }
        profileConnection = server.connectionId
    }

    private func selectionCurrent() -> Bool {
        guard let server, !profileConnection.isEmpty else { return false }
        return server.ready(token: settings.token(for: server.id)) && profileConnection == server.connectionId
    }

    func loadTargets() {
        guard let server, selectionCurrent() else {
            targets = []
            return
        }
        let serverId = server.id
        Task {
            do {
                let client = try APIClient(server: server)
                let accounts = try await client.accounts()
                guard selectedServerId == serverId else { return }
                targets = accounts.filter { $0.platform == "wx" }
                if !selectedTargetId.isEmpty, !targets.contains(where: { $0.id == selectedTargetId }) {
                    selectedTargetId = ""
                }
            } catch {
                guard selectedServerId == serverId else { return }
                targets = []
            }
        }
    }

    func start(openWeChat: Bool) {
        guard canStart else { return }
        guard selectionCurrent(), let server else {
            status = "所选服务器已删除或设置已变化，请重新选择「登录到」"
            detail = ""
            return
        }
        let targetId = selectedTargetId
        let base = baseName
        let actualName: String
        do {
            actualName = targetId.isEmpty ? try AccountNamePolicy.unique(base, nonce: nonce) : ""
        } catch {
            status = error.localizedDescription
            return
        }
        cancelTask()
        pollTask?.cancel()
        generation += 1
        let run = generation
        phase = .preparing
        status = "正在获取二维码…"
        detail = ""
        image = nil
        draft = nil
        pollFailures = 0

        Task {
            do {
                let api = try APIClient(server: server)
                guard profileConnection == api.connectionId else {
                    throw SettingsError.message("服务器配置已变化，请重新选择「登录到」")
                }
                let draft = try await api.prepareWxAccount(targetId: targetId, baseName: base, actualName: actualName)
                let task = try await api.createWxTask()
                let bytes = try await api.wxQr(task)
                guard run == generation else {
                    Task { try? await api.cancelWxTask(task) }
                    return
                }
                guard api.isCurrent else { throw SettingsError.message("服务器配置已变化，请重新开始") }
                let image = try QRCodeSaver.decode(bytes)
                self.client = api
                self.task = task
                self.draft = draft
                self.image = image
                self.phase = .waiting
                self.status = "等待扫码"
                self.detail = "请在另一台设备上显示二维码，用微信扫一扫。也可以保存图片后使用微信摄像头扫描。"
                if openWeChat { NotificationCenter.default.post(name: .wxOpenWeChat, object: nil) }
                schedulePoll(run)
            } catch {
                guard run == generation else { return }
                fail(error)
            }
        }
    }

    private func schedulePoll(_ run: Int) {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 1_200_000_000)
            guard let self, run == self.generation else { return }
            await self.poll(run)
        }
    }

    private func poll(_ run: Int) async {
        guard run == generation, phase == .waiting || phase == .scanned,
              let client, let task else { return }
        if Date() >= task.expiresAt {
            fail(SettingsError.message("二维码已过期，请重新获取；旧图片不能继续登录"))
            return
        }
        do {
            let state = try await client.wxStatus(task)
            guard run == generation else { return }
            pollFailures = 0
            switch state {
            case "authorized", "ready_for_code":
                phase = .authorized
                status = "已授权"
                detail = "点「完成登录」保存账号并上线。"
            case "waiting", "scanned":
                phase = state == "scanned" ? .scanned : .waiting
                status = state == "scanned" ? "已扫码，请在微信中确认" : "等待扫码"
                schedulePoll(run)
            default:
                phase = .failed
                status = "二维码已取消或失效，请重新获取"
                cancelTask()
            }
        } catch let error as APIError where error == .temporary {
            guard run == generation else { return }
            if Date() >= task.expiresAt {
                fail(SettingsError.message("二维码已过期，请重新获取；旧图片不能继续登录"))
                return
            }
            status = "网络中断，正在重试…"
            detail = "二维码仍有效，无需重新扫码。"
            pollFailures = min(pollFailures + 1, 3)
            let delay = min(8.0, pow(2.0, Double(pollFailures)))
            let remaining = max(1.0, task.expiresAt.timeIntervalSinceNow)
            try? await Task.sleep(nanoseconds: UInt64(min(delay, remaining) * 1_000_000_000))
            schedulePoll(run)
        } catch {
            guard run == generation else { return }
            fail(error)
        }
    }

    func finishLogin() {
        guard phase == .authorized, let client, let task, let draft else { return }
        if Date() >= task.expiresAt {
            fail(SettingsError.message("二维码已过期，请重新获取"))
            return
        }
        let run = generation
        phase = .finishing
        status = "正在登录…"
        detail = ""
        Task {
            do {
                let account = try await client.finishWxLogin(task: task, draft: draft)
                guard run == generation else { return }
                saved = account
                phase = .saved
                status = "登录成功"
                checkFriends()
            } catch let error as APIError where error == .uncertainWxSave {
                guard run == generation else { return }
                phase = .uncertain
                status = error.localizedDescription
                detail = "请先在账号页确认，不会自动重试。"
                cancelTask()
            } catch {
                guard run == generation else { return }
                fail(error)
            }
        }
    }

    func checkFriends() {
        guard phase == .saved, !checking, let client, let saved else { return }
        checking = true
        detail = "正在检查在线状态与好友…"
        Task {
            do {
                let accounts = try await client.accounts()
                guard let current = accounts.first(where: { $0.id == saved.id && $0.platform == "wx" }) else {
                    throw SettingsError.message("服务器账号已不存在")
                }
                guard try await client.connected(current) else {
                    throw SettingsError.message("账号已保存，尚未上线。稍后点「重新检查」。")
                }
                let count = try await client.wechatFriendCount(targetId: current.id)
                checking = false
                detail = count > 0 ? "已上线，读取到 \(count) 个有效好友" : "已上线，但未读取到好友"
            } catch {
                checking = false
                detail = error.localizedDescription
            }
        }
    }

    func reset() {
        guard phase != .finishing, phase != .uncertain else { return }
        generation += 1
        cancelTask()
        pollTask?.cancel()
        image = nil
        draft = nil
        saved = nil
        checking = false
        phase = .idle
        status = "准备就绪"
        detail = ""
    }

    private func fail(_ error: Error) {
        phase = .failed
        status = error.localizedDescription
        detail = ""
        cancelTask()
    }

    private func cancelTask() {
        guard let client, let task else {
            task = nil
            return
        }
        self.task = nil
        Task { try? await client.cancelWxTask(task) }
    }
}

extension Notification.Name {
    static let wxOpenWeChat = Notification.Name("FarmHelperOpenWeChat")
}
