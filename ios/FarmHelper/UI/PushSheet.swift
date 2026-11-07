import SwiftUI

struct PushSheet: View {
    let destination: PushDestination

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var pending: PendingChoice?
    @State private var createGroup: ServerGroup?
    @State private var accountName = ""

    private struct PendingChoice: Identifiable {
        let id = UUID()
        let choice: ServerDirectory.Choice
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    summaryCard
                    if let recommended = destination.recommended {
                        recommendationCard(recommended)
                    }
                    ForEach(destination.groups) { group in
                        groupCard(group)
                    }
                }
                .padding(20)
            }
            .background(Theme.background)
            .navigationTitle(destination.code ? "推送到哪个账号？" : "追加好友到")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { dismiss() }
                }
            }
            .alert(item: $pending) { pending in
                Alert(
                    title: Text("覆盖「\(pending.choice.account.name)」？"),
                    message: Text(pending.choice.account.identityLabel
                        + whereText(pending.choice.group)
                        + "\n这个账号的登录将被替换为本次获取的 Code。\n"
                        + summaryText),
                    primaryButton: .destructive(Text("覆盖")) {
                        Task { await model.push(choice: pending.choice, destination: destination) }
                    },
                    secondaryButton: .cancel(Text("取消"))
                )
            }
            .alert("新增账号", isPresented: createBinding) {
                TextField("账号备注", text: $accountName)
                Button("新增") {
                    let group = createGroup
                    let name = accountName
                    createGroup = nil
                    accountName = ""
                    if let group {
                        Task { await model.createAccount(group: group, baseName: name, destination: destination) }
                    }
                }
                Button("取消", role: .cancel) {
                    createGroup = nil
                    accountName = ""
                }
            } message: {
                Text("会自动附加随机后缀，避免与同名账号混淆。")
            }
        }
    }

    private var summaryCard: some View {
        Card {
            VStack(alignment: .leading, spacing: 8) {
                Text("本次推送").font(.system(size: 13, weight: .semibold)).foregroundStyle(Theme.accent)
                Text(summaryText).font(.system(size: 15))
                Text("写入前需要你确认目标账号；不会自动覆盖。")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.muted)
            }
            .padding(16)
        }
    }

    private func recommendationCard(_ choice: ServerDirectory.Choice) -> some View {
        Card {
            VStack(alignment: .leading, spacing: 8) {
                Text(destination.detected ? "检测到已有账号" : "上次推送的账号")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Theme.accent)
                Text(choice.account.name).font(.system(size: 17, weight: .semibold))
                Text(choice.account.identityLabel + whereText(choice.group))
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.muted)
                Button(destination.detected ? "更新这个账号" : "推送到这个账号") {
                    confirm(choice)
                }
                .buttonStyle(FilledButtonStyle())
                .padding(.top, 4)
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private func groupCard(_ group: ServerGroup) -> some View {
        let many = destination.groups.count > 1
        VStack(alignment: .leading, spacing: 8) {
            if many {
                Text(group.server.label)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Theme.accent)
                    .padding(.horizontal, 6)
            }
            Card {
                if !group.ok {
                    Text(group.error ?? "这个服务器暂时无法选择")
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.danger)
                        .padding(16)
                } else {
                    if group.accounts.isEmpty {
                        Text("没有可选的账号").font(.system(size: 14)).foregroundStyle(Theme.muted).padding(16)
                    }
                    ForEach(Array(group.accounts.indices), id: \.self) { index in
                        let account = group.accounts[index]
                        if index > 0 { DividerLine() }
                        Button {
                            confirm(ServerDirectory.Choice(group: group, account: account))
                        } label: {
                            HStack(spacing: 10) {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(account.name).font(.system(size: 15)).foregroundStyle(.primary)
                                    Text(account.identityLabel).font(.system(size: 12)).foregroundStyle(Theme.muted)
                                }
                                Spacer()
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 12, weight: .semibold))
                                    .foregroundStyle(Theme.muted)
                            }
                            .padding(.horizontal, 16)
                            .padding(.vertical, 11)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    if destination.code {
                        if !group.accounts.isEmpty { DividerLine() }
                        Button {
                            createGroup = group
                            accountName = ""
                        } label: {
                            HStack(spacing: 8) {
                                Image(systemName: "plus.circle.fill").foregroundStyle(Theme.accent)
                                Text(many ? "新增到这个服务器" : "新增账号")
                                    .foregroundStyle(Theme.accent)
                                Spacer()
                            }
                            .font(.system(size: 15, weight: .medium))
                            .padding(16)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private var summaryText: String {
        let count = destination.capture.gids.count
        if destination.code && destination.friends { return "将推送 Code 和 \(count) 位好友。" }
        if destination.code { return "将推送 Code。" }
        return "将追加 \(count) 位好友。"
    }

    private func whereText(_ group: ServerGroup) -> String {
        destination.groups.count > 1 ? "\n服务器：\(group.server.label)" : ""
    }

    private func confirm(_ choice: ServerDirectory.Choice) {
        if destination.code {
            pending = PendingChoice(choice: choice)
        } else {
            Task { await model.push(choice: choice, destination: destination) }
        }
    }

    private var createBinding: Binding<Bool> {
        Binding(
            get: { createGroup != nil },
            set: { value in if !value { createGroup = nil } }
        )
    }
}
