import SwiftUI
import UIKit

@MainActor
struct WxLoginView: View {
    @StateObject private var login = WxLoginModel()
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if login.servers.isEmpty {
                        Card {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("还没有可用的服务器").font(.system(size: 16, weight: .semibold))
                                Text("请先在设置中添加服务器与 Token。")
                                    .font(.system(size: 13))
                                    .foregroundStyle(Theme.muted)
                                Button("关闭") { dismiss() }
                                    .buttonStyle(TonalButtonStyle())
                            }
                            .padding(16)
                        }
                    } else {
                        setupCard
                        statusCard
                        if let image = login.image {
                            qrCard(image)
                        }
                        actionButtons
                    }

                    Text("二维码由服务器创建，有效期由服务器决定。应用不显示二维码之外的内容，也不会自动确认授权。")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 4)
                }
                .padding(20)
            }
            .background(Theme.background)
            .navigationTitle("微信扫码登录")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成") {
                        login.reset()
                        dismiss()
                    }
                }
            }
            .onAppear {
                login.load()
                if let preset = model.wxLoginPreset {
                    model.wxLoginPreset = nil
                    login.applyPreset(serverId: preset.serverId, targetId: preset.targetId)
                }
            }
            .onDisappear { login.reset() }
            .onReceive(NotificationCenter.default.publisher(for: .wxOpenWeChat)) { _ in
                openWeChat()
            }
        }
    }

    private var setupCard: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                Picker("登录到", selection: Binding(
                    get: { login.selectedServerId },
                    set: { login.selectServer($0) }
                )) {
                    ForEach(login.servers) { server in
                        Text(server.label).tag(server.id)
                    }
                }
                .pickerStyle(.menu)
                .disabled(!login.canStart)

                DividerLine()

                Picker("账号", selection: $login.selectedTargetId) {
                    Text("新增账号").tag("")
                    ForEach(login.targets) { account in
                        Text(account.selectionLabel).tag(account.id)
                    }
                }
                .pickerStyle(.menu)
                .disabled(!login.canStart)

                if login.selectedTargetId.isEmpty {
                    TextField("账号备注（自动附加随机后缀）", text: $login.baseName)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .disabled(!login.canStart)
                }
            }
            .padding(16)
        }
    }

    private var statusCard: some View {
        Card {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(phaseLabel).font(.system(size: 16, weight: .semibold))
                    Spacer()
                    if login.hasLiveQR, let task = login.taskEndText {
                        Text(task).font(.system(size: 11)).foregroundStyle(Theme.muted)
                    }
                }
                Text(login.status).font(.system(size: 14))
                if !login.detail.isEmpty {
                    Text(login.detail).font(.system(size: 12)).foregroundStyle(Theme.muted)
                }
            }
            .padding(16)
        }
    }

    private func qrCard(_ image: UIImage) -> some View {
        Card {
            VStack {
                Image(uiImage: image)
                    .resizable()
                    .interpolation(.none)
                    .scaledToFit()
                    .frame(width: 240, height: 240)
                    .padding(14)
                    .background(Color.white)
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                Text("用微信扫一扫；本机微信可能拒绝从相册识别此登录码。")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.muted)
                    .multilineTextAlignment(.center)
                    .padding(.top, 8)
            }
            .frame(maxWidth: .infinity)
            .padding(16)
        }
    }

    @ViewBuilder
    private var actionButtons: some View {
        if login.canStart {
            Button {
                login.start(openWeChat: false)
            } label: {
                Label("获取二维码", systemImage: "qrcode")
            }
            .buttonStyle(FilledButtonStyle())

            Button {
                login.start(openWeChat: true)
            } label: {
                Label("获取并打开微信", systemImage: "arrow.up.forward.app")
            }
            .buttonStyle(TonalButtonStyle())
        }

        if login.hasLiveQR {
            Button {
                saveQR()
            } label: {
                Label("保存二维码到相册", systemImage: "square.and.arrow.down")
            }
            .buttonStyle(TonalButtonStyle())

            Button {
                openWeChat()
            } label: {
                Label("打开微信", systemImage: "arrow.up.forward.app")
            }
            .buttonStyle(TonalButtonStyle())
        }

        if login.canFinish {
            Button {
                login.finishLogin()
            } label: {
                Label("完成登录", systemImage: "checkmark.circle")
            }
            .buttonStyle(FilledButtonStyle())
        }

        if login.phase == .saved {
            Button {
                login.checkFriends()
            } label: {
                Label(login.checking ? "正在检查…" : "重新检查在线与好友", systemImage: "arrow.clockwise")
            }
            .buttonStyle(TonalButtonStyle())
            .disabled(login.checking)
        }

        if login.phase != .idle, login.phase != .finishing, login.phase != .uncertain {
            Button("重置本次扫码") { login.reset() }
                .buttonStyle(CompactButtonStyle(tint: Theme.muted, background: Color(uiColor: .tertiarySystemFill)))
                .frame(maxWidth: .infinity)
        }
    }

    private var phaseLabel: String {
        switch login.phase {
        case .idle: return "准备就绪"
        case .preparing: return "正在获取二维码"
        case .waiting: return "等待扫码"
        case .scanned: return "已扫码"
        case .authorized: return "已授权"
        case .finishing: return "正在登录"
        case .saved: return "登录成功"
        case .failed: return "未完成"
        case .uncertain: return "结果待确认"
        }
    }

    private func saveQR() {
        guard let image = login.image else { return }
        Task {
            do {
                try await QRCodeSaver.save(image)
                model.showToast("二维码已保存到相册")
            } catch {
                model.showToast(error.localizedDescription)
            }
        }
    }

    private func openWeChat() {
        guard let url = URL(string: "weixin://") else { return }
        UIApplication.shared.open(url) { success in
            if !success {
                Task { @MainActor in model.showToast("未安装微信") }
            }
        }
    }
}
