import SwiftUI

struct LogsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var detailsExpanded = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Card {
                        Text(model.logs)
                            .font(.system(size: 12, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(16)
                    }
                    .frame(minHeight: 260, alignment: .top)

                    Card {
                        DisclosureGroup(isExpanded: $detailsExpanded) {
                            Text(model.tunnelStatus?.connectionText ?? "VPN 未运行。启动获取后这里会显示 TLS 主机与转发统计。")
                                .font(.system(size: 12, design: .monospaced))
                                .foregroundStyle(Theme.muted)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.top, 10)
                        } label: {
                            Text("连接详情").font(.system(size: 15, weight: .medium))
                        }
                        .tint(Theme.accent)
                        .padding(16)
                    }

                    Text("日志仅保留最近 200 条，写入前已脱敏；不保存原始包、Code、Token 或好友 GID。")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 6)
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 30)
            }
            .background(Theme.background)
            .navigationTitle("日志")
            .toolbar {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    ShareLink(item: model.logs) {
                        Image(systemName: "square.and.arrow.up")
                    }
                    Button(role: .destructive) {
                        model.clearLogs()
                    } label: {
                        Image(systemName: "trash")
                    }
                }
            }
        }
    }
}
