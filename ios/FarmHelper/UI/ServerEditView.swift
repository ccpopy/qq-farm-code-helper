import SwiftUI

struct ServerEditView: View {
    let existing: ServerProfile?

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var address: String
    @State private var token = ""
    @State private var allowHTTP: Bool
    @State private var saving = false
    @State private var errorText: String?
    @State private var showHTTPAlert = false
    @State private var showDeleteAlert = false

    init(existing: ServerProfile?) {
        self.existing = existing
        _name = State(initialValue: existing?.name ?? "")
        _address = State(initialValue: existing?.baseURL ?? "")
        _allowHTTP = State(initialValue: existing?.allowHTTP ?? false)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("服务器") {
                    TextField("名称（可选）", text: $name)
                    TextField("https://farm.example.com", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField(
                        existing == nil ? "管理 Token" : "管理 Token（已保存，留空不变）",
                        text: $token
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    Toggle("允许 HTTP（明文）", isOn: $allowHTTP)
                }

                if let errorText {
                    Section {
                        Text(errorText)
                            .font(.system(size: 13))
                            .foregroundStyle(Theme.danger)
                    }
                }

                Section {
                    Text("地址与 Token 只用于连接你自己的服务器。HTTP 会明文传输管理 Token 和 Code，建议使用 HTTPS。")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                }

                if let existing {
                    Section {
                        Button("删除这个服务器", role: .destructive) {
                            showDeleteAlert = true
                        }
                    } footer: {
                        Text("会删除这里保存的地址、Token 和掉线提醒选择。服务器上的账号不受影响。")
                    }
                    .disabled(existing.id.isEmpty)
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.background)
            .navigationTitle(existing == nil ? "添加服务器" : "编辑服务器")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { dismiss() }
                        .disabled(saving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(saving ? "保存中…" : "保存并测试") {
                        save()
                    }
                    .disabled(saving || address.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onChange(of: allowHTTP) { value in
                if value { showHTTPAlert = true }
            }
            .alert("允许 HTTP？", isPresented: $showHTTPAlert) {
                Button("取消", role: .cancel) { allowHTTP = false }
                Button("允许") {}
            } message: {
                Text("HTTP 会明文传输管理 Token 和 Code，仅在你完全信任网络环境时使用。")
            }
            .confirmationDialog("删除「\(existing?.label ?? "")」？", isPresented: $showDeleteAlert, titleVisibility: .visible) {
                Button("删除", role: .destructive) {
                    if let existing {
                        model.deleteServer(existing)
                    }
                    dismiss()
                }
                Button("取消", role: .cancel) {}
            } message: {
                Text("服务器上的账号不受影响；本机保存的 Token 与提醒选择会被删除。")
            }
            .interactiveDismissDisabled(saving)
        }
    }

    private func save() {
        saving = true
        errorText = nil
        Task {
            do {
                _ = try await model.saveServer(
                    existing: existing,
                    name: name,
                    address: address,
                    token: token,
                    allowHTTP: allowHTTP
                )
                dismiss()
            } catch {
                errorText = error.localizedDescription
                saving = false
            }
        }
    }
}
