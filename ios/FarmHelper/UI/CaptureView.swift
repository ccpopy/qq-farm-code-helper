import SwiftUI
import UIKit

struct CaptureView: View {
    @EnvironmentObject private var model: AppModel
    @State private var wechatVPN = false
    @State private var pulse = false
    @State private var showQuickPanel = false

    private var platform: CapturePlatform { model.settings.capturePlatform }
    private var running: Bool { model.vpnState == .connected || model.vpnState == .connecting }
    private var scanMode: Bool { platform == .wx && !wechatVPN }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 12) {
                    platformPicker
                    if platform == .wx { wechatModePicker }
                    banners
                    hero
                    if scanMode {
                        Button {
                            if running { model.showToast("请先停止 VPN 获取") }
                            else { model.showWxLogin = true }
                        } label: {
                            Label("打开微信扫码登录", systemImage: "qrcode")
                        }
                        .buttonStyle(FilledButtonStyle())
                        .disabled(running)
                    } else {
                        actionRow
                    }
                    resultCard
                    hints
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
            .background(Theme.background)
            .navigationTitle("农场助手")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showQuickPanel = true
                    } label: {
                        Image(systemName: "rectangle.on.rectangle")
                    }
                    .disabled(model.capture == nil && !running)
                    .accessibilityLabel("快捷面板")
                }
            }
            .sheet(isPresented: $showQuickPanel) {
                QuickPanelView()
                    .environmentObject(model)
                    .presentationDetents([.height(330)])
                    .presentationDragIndicator(.visible)
            }
            .onChange(of: running) { _ in
                updatePulse()
            }
            .onAppear { updatePulse() }
        }
    }

    private var platformPicker: some View {
        Picker("应用", selection: Binding(
            get: { platform },
            set: { model.switchPlatform($0) }
        )) {
            ForEach(CapturePlatform.allCases) { item in
                Text(item.label).tag(item)
            }
        }
        .pickerStyle(.segmented)
        .disabled(running)
        .padding(.top, 4)
    }

    private var wechatModePicker: some View {
        Picker("微信模式", selection: $wechatVPN) {
            Text("扫码登录").tag(false)
            Text("VPN 获取").tag(true)
        }
        .pickerStyle(.segmented)
        .disabled(running)
    }

    @ViewBuilder
    private var banners: some View {
        if model.settings.servers.isEmpty {
            Banner(title: "还没有添加服务器", actionTitle: "去添加") {
                model.selectedTab = 3
            }
        } else if !scanMode, !model.settings.passthrough, !model.certExists {
            Banner(title: "首次使用需要安装证书", actionTitle: "安装") {
                model.showCertificate = true
            }
        } else if !scanMode, !model.settings.passthrough, model.certTrusted == false {
            Banner(title: "证书已生成，尚未被系统信任", actionTitle: "查看") {
                model.showCertificate = true
            }
        }
    }

    private var hero: some View {
        VStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(Theme.accentSoft)
                    .frame(width: 176, height: 176)
                    .scaleEffect(pulse ? 1.06 : 0.86)
                    .opacity(pulse ? 0.45 : 1)
                Button(action: primaryAction) {
                    ZStack {
                        Circle()
                            .fill(buttonFill)
                            .frame(width: 128, height: 128)
                            .shadow(color: Color.black.opacity(0.08), radius: 8, y: 4)
                        Image(systemName: buttonIcon)
                            .font(.system(size: 44, weight: .medium))
                            .foregroundStyle(buttonInk)
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel(buttonLabel)
            }
            .frame(height: 200)
            .padding(.top, 4)

            Text(buttonLabel)
                .font(.system(size: 20, weight: .semibold))
            Text(heroStatus)
                .font(.system(size: 13))
                .foregroundStyle(Theme.muted)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 8)
            if scanMode {
                Text("通过服务器生成登录二维码")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.muted)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 18)
        .background(
            LinearGradient(
                colors: [Theme.accentSoft.opacity(0.7), Theme.card],
                startPoint: .top, endPoint: .bottom
            )
        )
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }

    private var actionRow: some View {
        HStack(spacing: 10) {
            Button {
                model.openApp(platform.urlScheme)
            } label: {
                Label("打开 \(platform.label)", systemImage: "arrow.up.forward.app")
            }
            .buttonStyle(TonalButtonStyle())
            .disabled(running && model.tunnelStatus?.ready != true)

            Button {
                model.showManualImport = true
            } label: {
                Label("手动导入", systemImage: "square.and.pencil")
            }
            .buttonStyle(TonalButtonStyle())
        }
    }

    @ViewBuilder
    private var resultCard: some View {
        if let capture = model.capture, capture.hasPending || capture.friendsObserved || !capture.consumedGids.isEmpty {
            Card {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("本次结果").font(.system(size: 16, weight: .semibold))
                        Spacer()
                        Text(capture.platform.label)
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(Theme.accent)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 3)
                            .background(Theme.accentSoft)
                            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                    }
                    HStack(spacing: 8) {
                        StatTile(value: capture.code.isEmpty ? "—" : "已获取", label: "Code")
                        StatTile(value: "\(capture.gids.count)", label: "待同步好友")
                        StatTile(value: capture.ownGid.isEmpty ? "—" : "已识别", label: "身份")
                    }
                    HStack(spacing: 10) {
                        Button {
                            model.preparePush()
                        } label: {
                            Label("推送到服务器", systemImage: "arrow.up.circle")
                        }
                        .buttonStyle(FilledButtonStyle())
                        .disabled(model.busy || (!capture.hasPending))

                        Button("清除") {
                            model.clearCapture()
                        }
                        .buttonStyle(CompactButtonStyle(tint: Theme.danger, background: Theme.dangerSoft))
                        .disabled(model.busy)
                    }
                }
                .padding(16)
            }
        }
    }

    private var hints: some View {
        VStack(alignment: .leading, spacing: 8) {
            if model.settings.passthrough {
                Label("透传诊断：不解密，也不会获取 Code 或好友。", systemImage: "exclamationmark.triangle")
            }
            if platform == .wx {
                Label(platform.compatibilityHint, systemImage: "info.circle")
            }
            Label("若进入农场超时，请暂时只保留一种网络连接，或关闭系统的网络加速／双通道加速后重试。", systemImage: "wifi.exclamationmark")
            Label("VPN 仅由你手动启动；停止后不会自行重启。", systemImage: "lock.shield")
        }
        .font(.system(size: 12))
        .foregroundStyle(Theme.muted)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    private var buttonFill: Color {
        if scanMode { return Theme.card }
        return (running || model.capture != nil) ? Theme.accent : Theme.card
    }

    private var buttonInk: Color {
        if scanMode { return Theme.accent }
        return (running || model.capture != nil) ? .white : Theme.accent
    }

    private var buttonIcon: String {
        if scanMode { return "qrcode" }
        if running { return "stop.fill" }
        if model.capture != nil, model.vpnState != .connected { return "checkmark" }
        return "power"
    }

    private var buttonLabel: String {
        if scanMode { return "扫码登录" }
        if running { return "停止 VPN" }
        if model.settings.passthrough { return "开始透传诊断" }
        return "开始获取"
    }

    private var heroStatus: String {
        if scanMode { return "通过服务器生成登录二维码" }
        if running || model.capture != nil { return model.captureStatus }
        if model.settings.passthrough { return "只测试联网，不获取 Code" }
        return "点击后进入 \(platform.label) 农场"
    }

    private func primaryAction() {
        if scanMode {
            model.showWxLogin = true
        } else {
            model.toggleTunnel()
        }
    }

    private func updatePulse() {
        if running {
            withAnimation(.easeInOut(duration: 1.3).repeatForever(autoreverses: true)) {
                pulse = true
            }
        } else {
            pulse = false
        }
    }
}

struct QuickPanelView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    private var running: Bool { model.vpnState == .connected || model.vpnState == .connecting }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("快捷面板").font(.system(size: 20, weight: .semibold))
                Spacer()
                Button("收起") { dismiss() }
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Theme.accent)
            }
            Text(model.captureStatus)
                .font(.system(size: 14))
                .foregroundStyle(Theme.muted)
            if let capture = model.capture {
                HStack(spacing: 8) {
                    StatTile(value: capture.code.isEmpty ? "—" : "已获取", label: "Code")
                    StatTile(value: "\(capture.gids.count)", label: "待同步好友")
                    StatTile(value: capture.ownGid.isEmpty ? "—" : "已识别", label: "身份")
                }
            }
            HStack(spacing: 10) {
                Button {
                    model.preparePush()
                } label: {
                    Text("同步")
                }
                .buttonStyle(FilledButtonStyle())
                .disabled(model.capture == nil || model.busy)

                Button {
                    model.clearCapture()
                } label: {
                    Text("清除")
                }
                .buttonStyle(CompactButtonStyle(tint: Theme.danger, background: Theme.dangerSoft))
                .disabled(model.busy)
            }
            HStack(spacing: 10) {
                Button {
                    model.toggleTunnel()
                } label: {
                    Text(running ? "停止 VPN" : "开始 VPN")
                }
                .buttonStyle(TonalButtonStyle())

                Button("关闭") { dismiss() }
                    .buttonStyle(TonalButtonStyle(background: Color(uiColor: .tertiarySystemFill)))
            }
            Spacer(minLength: 0)
        }
        .padding(20)
        .background(Theme.background)
    }
}
