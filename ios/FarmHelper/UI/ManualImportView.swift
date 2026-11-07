import SwiftUI
import UIKit

struct ManualImportView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var code = ""
    @State private var gids = ""
    @State private var errorText: String?

    private var platform: CapturePlatform { model.settings.capturePlatform }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text("手动导入 · \(platform.label)")
                        .font(.system(size: 16, weight: .semibold))

                    Button {
                        if let text = UIPasteboard.general.string { code = text }
                    } label: {
                        Label("从剪贴板粘贴 Code", systemImage: "doc.on.clipboard")
                    }
                    .buttonStyle(TonalButtonStyle())

                    Card {
                        VStack(alignment: .leading, spacing: 10) {
                            TextField("Code 或登录链接", text: $code, axis: .vertical)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .lineLimit(1...4)
                            DividerLine()
                            ZStack(alignment: .topLeading) {
                                if gids.isEmpty {
                                    Text("好友 GID（可选，逗号或换行分隔）")
                                        .font(.system(size: 15))
                                        .foregroundStyle(Color(uiColor: .placeholderText))
                                        .padding(.top, 8)
                                        .padding(.leading, 4)
                                }
                                TextEditor(text: $gids)
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                                    .frame(minHeight: 110)
                                    .scrollContentBackground(.hidden)
                            }
                        }
                        .padding(16)
                    }

                    if let errorText {
                        Text(errorText)
                            .font(.system(size: 13))
                            .foregroundStyle(Theme.danger)
                    }

                    Button {
                        do {
                            try model.manualImport(codeText: code, gidsText: gids)
                            dismiss()
                        } catch {
                            errorText = error.localizedDescription
                        }
                    } label: {
                        Label("导入到本机结果", systemImage: "square.and.arrow.down")
                    }
                    .buttonStyle(FilledButtonStyle())

                    Text("手动导入只写入本机结果，仍需在结果卡片中确认后推送。只填写好友 GID 时不能新增账号。")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                }
                .padding(20)
            }
            .background(Theme.background)
            .navigationTitle("手动导入")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { dismiss() }
                }
            }
        }
    }
}
