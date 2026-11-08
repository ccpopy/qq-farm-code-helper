import SwiftUI
import UIKit

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var editingServer: ServerProfile?
    @State private var addingServer = false
    @State private var mirrorText = ""

    private var running: Bool { model.vpnState == .connected || model.vpnState == .connecting }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    SectionLabel("服务器")
                    serversCard

                    SectionLabel("推送")
                    Card {
                        ToggleRow("获取后自动推送", subtitle: "自动比对服务器账号，确认后推送", isOn: binding(\.autoPush))
                        DividerLine()
                        ToggleRow("推送 Code", isOn: binding(\.pushCode))
                        DividerLine()
                        ToggleRow("推送好友", isOn: binding(\.pushFriends))
                    }

                    SectionLabel("获取")
                    Card {
                        ToggleRow(
                            model.settings.capturePlatform == .wx ? "微信 VPN 只读好友" : "同时读取好友",
                            subtitle: model.settings.capturePlatform == .wx ? "开启后不保留 Code" : nil,
                            isOn: Binding(
                                get: { model.settings.friendCapture },
                                set: { value in
                                    model.updateSettings { $0.friendCapture = value }
                                    if running { model.showToast("重新开始获取后生效") }
                                }
                            )
                        )
                        DividerLine()
                        ToggleRow(
                            "透传诊断",
                            subtitle: "不解密，只测试联网",
                            isOn: Binding(
                                get: { model.settings.passthrough },
                                set: { value in
                                    model.updateSettings { $0.passthrough = value }
                                    if running { model.showToast("重新开始获取后生效") }
                                }
                            )
                        )
                        DividerLine()
                        Button {
                            model.showCertificate = true
                        } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text("抓包证书").font(.system(size: 15)).foregroundStyle(.primary)
                                    Text(certificateState).font(.system(size: 12)).foregroundStyle(Theme.muted)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.muted)
                            }
                            .padding(16)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }

                    SectionLabel("提醒")
                    Card {
                        Stepper(
                            value: Binding(
                                get: { model.settings.pollSeconds },
                                set: { value in model.updateSettings { $0.pollSeconds = value } }
                            ),
                            in: 1...3600
                        ) {
                            HStack {
                                Text("检查间隔").font(.system(size: 15))
                                Spacer()
                                Text("\(model.settings.pollSeconds) 秒").foregroundStyle(Theme.muted)
                            }
                        }
                        .padding(16)
                        DividerLine()
                        Button {
                            if let url = URL(string: UIApplication.openSettingsURLString) {
                                UIApplication.shared.open(url)
                            }
                        } label: {
                            HStack {
                                Text("通知与系统设置").font(.system(size: 15)).foregroundStyle(.primary)
                                Spacer()
                                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.muted)
                            }
                            .padding(16)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        DividerLine()
                        Text("VPN 运行时由隧道扩展执行长轮询；未运行时由系统后台刷新调度，可能出现延迟。")
                            .font(.system(size: 12))
                            .foregroundStyle(Theme.muted)
                            .padding(16)
                    }

                    SectionLabel("平台限制")
                    Card {
                        limitationRow("悬浮窗", "iOS 不允许跨应用悬浮窗，已用应用内快捷面板替代。")
                        DividerLine()
                        limitationRow("截屏保护", "iOS 不允许第三方应用阻止系统截屏。")
                        DividerLine()
                        limitationRow("后台运行", "VPN 运行时由扩展保持；停止 VPN 后应用会被系统挂起。")
                        DividerLine()
                        limitationRow("证书信任", "导出证书后需在系统设置中安装描述文件，并打开“完全信任”。")
                    }

                    SectionLabel("更新")
                    Card {
                        Button {
                            Task { await model.checkUpdates() }
                        } label: {
                            HStack {
                                Text("检查更新").font(.system(size: 15)).foregroundStyle(.primary)
                                Spacer()
                                Text("当前 v\(appVersion)").font(.system(size: 12)).foregroundStyle(Theme.muted)
                            }
                            .padding(16)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        DividerLine()
                        ToggleRow("下载加速", isOn: binding(\.accelerated))
                        DividerLine()
                        HStack(spacing: 10) {
                            TextField("https://ghfast.top", text: $mirrorText)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .font(.system(size: 14))
                            Button("保存") { saveMirror() }
                                .buttonStyle(CompactButtonStyle())
                        }
                        .padding(16)
                    }

                    SectionLabel("关于")
                    Card {
                        Button {
                            model.openURL("https://github.com/ccpopy/qq-farm-code-helper")
                        } label: {
                            HStack {
                                Text("源代码").font(.system(size: 15)).foregroundStyle(.primary)
                                Spacer()
                                Image(systemName: "arrow.up.right").font(.system(size: 13)).foregroundStyle(Theme.muted)
                            }
                            .padding(16)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        DividerLine()
                        Text("农场助手 iOS。仅用于用户自己的服务器与账号；不读取聊天、通讯录或存储文件。微信 VPN 获取仍为实验性功能。")
                            .font(.system(size: 12))
                            .foregroundStyle(Theme.muted)
                            .padding(16)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 34)
            }
            .background(Theme.background)
            .navigationTitle("设置")
            .sheet(item: $editingServer) { server in
                ServerEditView(existing: server)
                    .environmentObject(model)
            }
            .sheet(isPresented: $addingServer) {
                ServerEditView(existing: nil)
                    .environmentObject(model)
            }
            .onAppear {
                mirrorText = model.settings.mirror
                model.pingServers()
            }
        }
    }

    private var serversCard: some View {
        Card {
            ForEach(model.settings.servers) { server in
                if server.id != model.settings.servers.first?.id { DividerLine() }
                Button {
                    editingServer = server
                } label: {
                    HStack(spacing: 12) {
                        StatusDot(color: serverColor(server))
                        VStack(alignment: .leading, spacing: 3) {
                            Text(server.label).font(.system(size: 15)).foregroundStyle(.primary)
                            Text(serverSubtitle(server)).font(.system(size: 12)).foregroundStyle(Theme.muted)
                        }
                        Spacer()
                        Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.muted)
                    }
                    .padding(16)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            if !model.settings.servers.isEmpty { DividerLine() }
            Button {
                addingServer = true
            } label: {
                HStack(spacing: 10) {
                    Image(systemName: "plus.circle.fill").foregroundStyle(Theme.accent)
                    Text("添加服务器").foregroundStyle(Theme.accent)
                    Spacer()
                }
                .font(.system(size: 15, weight: .medium))
                .padding(16)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    private func limitationRow(_ title: String, _ detail: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(.system(size: 15))
            Text(detail).font(.system(size: 12)).foregroundStyle(Theme.muted)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
    }

    private func binding(_ keyPath: WritableKeyPath<AppSettings, Bool>) -> Binding<Bool> {
        Binding(
            get: { model.settings[keyPath: keyPath] },
            set: { value in model.updateSettings { $0[keyPath: keyPath] = value } }
        )
    }

    private func serverColor(_ server: ServerProfile) -> Color {
        guard server.ready(token: SharedSettings.shared.token(for: server.id)) else { return Theme.hairline }
        if let error = model.serverErrors[server.id], !error.isEmpty { return Theme.danger }
        return Theme.accent
    }

    private func serverSubtitle(_ server: ServerProfile) -> String {
        guard server.ready(token: SharedSettings.shared.token(for: server.id)) else { return "未填写 Token" }
        if let error = model.serverErrors[server.id], !error.isEmpty { return server.host + " · " + error }
        return server.host
    }

    private var certificateState: String {
        if !model.certExists { return "未生成" }
        switch model.certTrusted {
        case .some(true): return "已安装并信任"
        case .some(false): return "已生成，尚未被系统信任"
        case .none: return "已生成，正在检查信任状态"
        }
    }

    private var appVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1.3"
    }

    private func saveMirror() {
        do {
            let base = try UrlPolicy.server(mirrorText, allowHTTP: false)
            model.updateSettings { $0.mirror = base }
            mirrorText = base
            model.showToast("加速地址已保存")
        } catch {
            model.showToast(error.localizedDescription)
        }
    }
}

struct UpdateView: View {
    let info: UpdateInfo
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text("v\(info.version)")
                        .font(.system(size: 24, weight: .semibold))
                    Text(info.notes.isEmpty ? "新版本已发布。" : info.notes)
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.muted)
                        .textSelection(.enabled)
                    Text("iOS 不能由应用自行安装更新。下载页会按你的签名与分发方式提供新版本。")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                    Button {
                        model.openURL(info.pageURL.absoluteString)
                    } label: {
                        Text("打开下载页")
                    }
                    .buttonStyle(FilledButtonStyle())
                }
                .padding(20)
            }
            .background(Theme.background)
            .navigationTitle("发现新版本")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") {
                        dismiss()
                        model.showUpdate = false
                    }
                }
            }
        }
    }
}
