import SwiftUI
import UIKit

struct AccountsView: View {
    @EnvironmentObject private var model: AppModel

    private var running: Bool { model.vpnState == .connected || model.vpnState == .connecting }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    monitorCard
                    if model.settings.servers.isEmpty {
                        emptyServers
                    } else {
                        ForEach(model.settings.servers) { server in
                            serverSection(server)
                        }
                    }
                    Text("点账号重新登录，点铃铛开关该账号的掉线提醒。")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 6)
                        .padding(.top, 4)
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 30)
            }
            .background(Theme.background)
            .navigationTitle("账号")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        model.refreshAccounts()
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(model.loadingAccounts)
                }
            }
            .refreshable { model.refreshAccounts() }
        }
    }

    private var monitorCard: some View {
        Card {
            ToggleRow(
                "掉线提醒",
                subtitle: monitorDetail,
                isOn: Binding(
                    get: { model.settings.monitorEnabled },
                    set: { model.setMonitorEnabled($0) }
                )
            )
        }
        .padding(.top, 4)
    }

    private var monitorDetail: String {
        let count = SharedSettings.shared.monitoredCount()
        if !model.settings.monitorEnabled {
            return count == 0 ? "已关闭" : "已关闭 · 已选 \(count) 个账号"
        }
        if count == 0 { return "已开启 · 未选择账号" }
        return running ? "正在提醒 \(count) 个账号" : "已开启 · VPN 未运行时由系统后台检查"
    }

    private var emptyServers: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                Text("添加服务器后，这里会显示账号和在线状态。")
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.muted)
                Button("添加服务器") { model.selectedTab = 3 }
                    .buttonStyle(TonalButtonStyle())
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private func serverSection(_ server: ServerProfile) -> some View {
        let group = model.groups.first { $0.id == server.id }
        let many = model.settings.servers.count > 1
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Text((many ? server.label : "服务器账号") + (group?.ok == true ? " · \(group?.accounts.count ?? 0)" : ""))
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(many ? Theme.accent : Theme.muted)
                Spacer()
                if let group, group.ok, !group.accounts.isEmpty {
                    Button(allSelected(group, server) ? "全不选" : "全选") {
                        model.selectAll(server: server, group: group)
                    }
                    .buttonStyle(CompactButtonStyle())
                }
            }
            .padding(.horizontal, 6)
            .padding(.top, 8)

            Card {
                if !server.ready(token: SharedSettings.shared.token(for: server.id)) {
                    Text("未填写 Token。点设置中的服务器行补全后即可读取账号。")
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.muted)
                        .padding(16)
                } else if let group {
                    if !group.ok {
                        HStack(spacing: 10) {
                            StatusDot(color: Theme.danger)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(group.error ?? "读取失败").font(.system(size: 15))
                                Text(server.host).font(.system(size: 12)).foregroundStyle(Theme.muted)
                            }
                            Spacer()
                            Button("重试") { model.retryAccounts(server) }
                                .buttonStyle(CompactButtonStyle())
                        }
                        .padding(16)
                    } else if group.accounts.isEmpty {
                        Text("还没有账号。获取 Code 后推送即可新增。")
                            .font(.system(size: 14))
                            .foregroundStyle(Theme.muted)
                            .padding(16)
                    } else {
                        ForEach(Array(group.accounts.indices), id: \.self) { index in
                            if index > 0 { DividerLine() }
                            accountRow(server: server, group: group, account: group.accounts[index])
                        }
                    }
                } else {
                    Text(model.loadingAccounts ? "正在读取…" : "尚未读取")
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.muted)
                        .padding(16)
                }
            }
        }
    }

    private func accountRow(server: ServerProfile, group: ServerGroup, account: Account) -> some View {
        let watched = server.monitoredIds.contains(account.id)
        let state = account.running ? model.onlineState(server: server, account: account) : nil
        let stateText = state == nil ? (account.running ? "检查中" : "已停止") : (state == true ? "在线" : "离线")
        let color = state == nil ? Theme.hairline : (state == true ? Theme.accent : Theme.danger)
        return HStack(spacing: 12) {
            Button {
                model.requestRelogin(group, account)
            } label: {
                HStack(spacing: 12) {
                    StatusDot(color: color)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(account.name).font(.system(size: 15)).foregroundStyle(.primary)
                        Text(account.identityLabel + " · " + stateText)
                            .font(.system(size: 12))
                            .foregroundStyle(Theme.muted)
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Button {
                model.toggleWatch(server: server, account: account)
            } label: {
                Image(systemName: watched ? "bell.fill" : "bell.slash")
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(watched ? Theme.accent : Theme.muted)
                    .frame(width: 40, height: 40)
                    .background(watched ? Theme.accentSoft : Color.clear)
                    .clipShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(watched ? "关闭提醒" : "开启提醒")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 9)
        .frame(minHeight: 56)
    }

    private func allSelected(_ group: ServerGroup, _ server: ServerProfile) -> Bool {
        let watched = Set(server.monitoredIds)
        return !group.accounts.isEmpty && group.accounts.allSatisfy { watched.contains($0.id) }
    }
}
