import SwiftUI

struct RootView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        TabView(selection: $model.selectedTab) {
            CaptureView()
                .tag(0)
                .tabItem { Label("获取", systemImage: "antenna.radiowaves.left.and.right") }
            AccountsView()
                .tag(1)
                .tabItem { Label("账号", systemImage: "person.2") }
            LogsView()
                .tag(2)
                .tabItem { Label("日志", systemImage: "doc.text") }
            SettingsView()
                .tag(3)
                .tabItem { Label("设置", systemImage: "gearshape") }
        }
        .tint(Theme.accent)
        .overlay(alignment: .bottom) {
            if let toast = model.toast {
                ToastView(text: toast)
                    .allowsHitTesting(false)
                    .animation(.easeInOut(duration: 0.2), value: toast)
            }
        }
        .sheet(item: $model.pushDestination) { destination in
            PushSheet(destination: destination)
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showWxLogin) {
            WxLoginView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showManualImport) {
            ManualImportView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showCertificate) {
            CertificateView()
                .environmentObject(model)
        }
        .sheet(isPresented: $model.showUpdate) {
            if let info = model.updateInfo {
                UpdateView(info: info)
                    .environmentObject(model)
            }
        }
        .alert(item: $model.reloginPrompt) { prompt in
            Alert(
                title: Text("重新登录「\(prompt.accountName)」？"),
                message: Text(prompt.platform == "wx"
                    ? "将打开微信扫码登录。"
                    : "请先在 QQ 中切换到这个账号，然后开始获取。"),
                primaryButton: .default(Text("继续")) {
                    Task { await model.confirmRelogin(prompt) }
                },
                secondaryButton: .cancel(Text("取消"))
            )
        }
    }
}
