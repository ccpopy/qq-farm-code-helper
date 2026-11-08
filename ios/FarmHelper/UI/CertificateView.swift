import SwiftUI

struct CertificateView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var profileURL: URL?
    @State private var certificateURL: URL?
    @State private var showDeleteAlert = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Card {
                        VStack(alignment: .leading, spacing: 10) {
                            HStack {
                                StatusDot(color: statusColor)
                                Text(statusText).font(.system(size: 16, weight: .semibold))
                                Spacer()
                            }
                            if model.certExists {
                                Text(model.certificateFingerprint)
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundStyle(Theme.muted)
                                    .textSelection(.enabled)
                            }
                            Text("本地 CA 只用于解密 \(UrlPolicy.farmHost) 的 TLS；其他流量原样转发。私钥保存在 Keychain，不会随描述文件导出。")
                                .font(.system(size: 12))
                                .foregroundStyle(Theme.muted)
                        }
                        .padding(16)
                    }

                    if !model.certExists {
                        Button {
                            Task { await model.generateCertificate() }
                        } label: {
                            Label("生成本地证书", systemImage: "key")
                        }
                        .buttonStyle(FilledButtonStyle())
                    } else {
                        Button {
                            profileURL = model.exportProfileURL()
                        } label: {
                            Label("生成安装描述文件", systemImage: "doc.badge.plus")
                        }
                        .buttonStyle(FilledButtonStyle())

                        if let profileURL {
                            ShareLink(item: profileURL) {
                                Label("分享描述文件到“文件”", systemImage: "square.and.arrow.up")
                            }
                            .buttonStyle(TonalButtonStyle())
                        }

                        Button {
                            certificateURL = model.exportCertificateURL()
                        } label: {
                            Label("导出 .cer 原件", systemImage: "square.and.arrow.down")
                        }
                        .buttonStyle(TonalButtonStyle())

                        if let certificateURL {
                            ShareLink(item: certificateURL) {
                                Label("分享 .cer", systemImage: "square.and.arrow.up")
                            }
                            .buttonStyle(TonalButtonStyle())
                        }

                        Button {
                            model.refreshCertificateTrust()
                        } label: {
                            Label("重新检查信任状态", systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(TonalButtonStyle())
                    }

                    Card {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("安装步骤").font(.system(size: 16, weight: .semibold))
                            step(1, "点“生成安装描述文件”，在分享面板中选择“存储到文件”。")
                            step(2, "打开“文件”App，点这个 .mobileconfig，按提示到“设置”中安装描述文件。")
                            step(3, "打开 设置 › 通用 › 关于本机 › 证书信任设置，为“Farm Helper Local CA”打开完全信任。")
                            step(4, "返回应用点“重新检查信任状态”，确认显示“已安装并信任”。")
                            Text("安装描述文件不等于已信任；必须手动打开完全信任。删除描述文件不会删除 Keychain 中的私钥。")
                                .font(.system(size: 12))
                                .foregroundStyle(Theme.muted)
                        }
                        .padding(16)
                    }

                    if model.certExists {
                        Button("删除本地证书与私钥", role: .destructive) {
                            showDeleteAlert = true
                        }
                        .buttonStyle(CompactButtonStyle(tint: Theme.danger, background: Theme.dangerSoft))
                        .frame(maxWidth: .infinity)
                    }
                }
                .padding(20)
            }
            .background(Theme.background)
            .navigationTitle("抓包证书")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成") { dismiss() }
                }
            }
            .alert("删除本地证书？", isPresented: $showDeleteAlert) {
                Button("删除", role: .destructive) {
                    Task { await model.deleteCertificate() }
                }
                Button("取消", role: .cancel) {}
            } message: {
                Text("证书私钥将被永久删除，重新生成后需要重新安装。系统中已安装的描述文件需另行移除。")
            }
            .onAppear {
                model.refreshCertificateTrust()
            }
        }
    }

    private var statusText: String {
        if !model.certExists { return "未生成" }
        switch model.certTrusted {
        case .some(true): return "已安装并信任"
        case .some(false): return "已生成，尚未被系统信任"
        case .none: return "已生成，正在检查信任状态"
        }
    }

    private var statusColor: Color {
        if !model.certExists { return Theme.hairline }
        return model.certTrusted == true ? Theme.accent : Theme.amber
    }

    private func step(_ number: Int, _ text: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text("\(number).")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.accent)
                .frame(width: 18, alignment: .leading)
            Text(text)
                .font(.system(size: 13))
                .foregroundStyle(Theme.muted)
        }
    }
}
