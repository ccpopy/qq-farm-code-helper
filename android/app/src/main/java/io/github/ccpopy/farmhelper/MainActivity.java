package io.github.ccpopy.farmhelper;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Typeface;
import android.graphics.Rect;
import android.net.Uri;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import androidx.annotation.Nullable;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.content.FileProvider;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowCompat;
import androidx.core.app.NotificationManagerCompat;
import com.google.android.material.bottomnavigation.BottomNavigationView;
import com.google.android.material.bottomsheet.BottomSheetDialog;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.card.MaterialCardView;
import com.google.android.material.dialog.MaterialAlertDialogBuilder;
import com.google.android.material.materialswitch.MaterialSwitch;
import com.google.android.material.textfield.TextInputEditText;
import com.google.android.material.textfield.TextInputLayout;
import java.io.File;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

public final class MainActivity extends AppCompatActivity {
    private LinearLayout root, content;
    private ScrollView scroll;
    private BottomNavigationView navigation;
    private TextView status, captureSummary, target, logs, monitorStatus;
    private MaterialButton vpnButton, monitorButton, qqButton;
    private TextView connections;
    private int page = 1;
    private final Runnable listener = this::refresh;
    private Runnable afterPermission;
    private String pendingExport;
    private File verifiedUpdate;
    private boolean busy;

    @Override protected void onCreate(@Nullable Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        root = column(); root.setBackgroundColor(getColor(R.color.farm_surface));
        root.setId(R.id.screen_root); root.setFocusableInTouchMode(true);
        ViewCompat.setOnApplyWindowInsetsListener(root, (view, insets) -> {
            Insets bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
            Insets keyboard = insets.getInsets(WindowInsetsCompat.Type.ime());
            boolean editing = insets.isVisible(WindowInsetsCompat.Type.ime());
            view.setPadding(bars.left, bars.top, bars.right, Math.max(bars.bottom, keyboard.bottom));
            navigation.setVisibility(editing ? View.GONE : View.VISIBLE);
            if (editing) scroll.post(() -> {
                View focus = getCurrentFocus();
                if (focus != null && focus != root) focus.requestRectangleOnScreen(new Rect(0, 0, focus.getWidth(), focus.getHeight() + dp(12)), false);
            });
            // Root owns edge/IME padding; do not let Material navigation add it a second time.
            return WindowInsetsCompat.CONSUMED;
        });
        scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setClipToPadding(false);
        scroll.setId(R.id.screen_scroll);
        content = column(); content.setPadding(dp(20), dp(12), dp(20), dp(24)); scroll.addView(content);
        root.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        navigation = new BottomNavigationView(this); navigation.setBackgroundColor(getColor(R.color.farm_card));
        navigation.setId(R.id.bottom_navigation);
        navigation.setLabelVisibilityMode(BottomNavigationView.LABEL_VISIBILITY_LABELED);
        navigation.getMenu().add(0, R.id.nav_capture, 0, "抓取").setIcon(R.drawable.ic_capture);
        navigation.getMenu().add(0, R.id.nav_accounts, 1, "账号").setIcon(R.drawable.ic_accounts);
        navigation.getMenu().add(0, R.id.nav_logs, 2, "日志").setIcon(R.drawable.ic_logs);
        navigation.getMenu().add(0, R.id.nav_settings, 3, "设置").setIcon(R.drawable.ic_settings);
        navigation.setOnItemSelectedListener(item -> { page = item.getOrder() + 1; showPage(); return true; });
        root.addView(navigation, new LinearLayout.LayoutParams(-1, -2)); setContentView(root);
        ViewCompat.requestApplyInsets(root);
        if (savedInstanceState != null) page = savedInstanceState.getInt("page", 1);
        navigation.setSelectedItemId(navId(page)); showPage();
        handleIntent(getIntent());
    }
    @Override protected void onResume() { super.onResume(); AppState.listen(listener); refresh(); }
    @Override protected void onPause() { AppState.unlisten(listener); super.onPause(); }
    @Override protected void onSaveInstanceState(Bundle state) { state.putInt("page", page); super.onSaveInstanceState(state); }
    @Override protected void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); handleIntent(intent); }

    private void handleIntent(Intent intent) {
        if (intent.getBooleanExtra("showSettings", false)) navigation.setSelectedItemId(R.id.nav_settings);
        intent.removeExtra("showSettings");
        if (Intent.ACTION_SEND.equals(intent.getAction()) && "text/plain".equals(intent.getType())) {
            String text = intent.getStringExtra(Intent.EXTRA_TEXT);
            if (text != null && text.length() <= 16384) root.post(() -> manual(text));
            intent.removeExtra(Intent.EXTRA_TEXT); intent.setAction(Intent.ACTION_MAIN);
        }
        String id = intent.getStringExtra("accountId"), server = intent.getStringExtra("server");
        String connection = intent.getStringExtra("connectionId");
        intent.removeExtra("accountId"); intent.removeExtra("server"); intent.removeExtra("connectionId");
        if (id != null && !id.isEmpty() && (!AppState.settings.server().equals(server)
                || connection == null || connection.isEmpty() || !connection.equals(AppState.settings.connectionId()))) {
            message("服务器配置已变化，请重新选择账号；旧通知不会自动绑定"); return;
        }
        if (id != null && !id.isEmpty()) {
            async("正在确认通知对应的账号", () -> {
                for (ApiClient.Account account : new ApiClient(AppState.settings).accounts()) if (account.id.equals(id)) {
                    if (!account.platform.equals("qq")) throw new IllegalStateException("该账号不是 QQ，当前不能在此抓取");
                    runOnUiThread(() -> new MaterialAlertDialogBuilder(this).setTitle("为这个账号重新获取？")
                            .setMessage(account.name + "\n服务器账号 ID：" + account.id + "\n请先在 QQ 中切换到同一个账号。通知不会自动启动 VPN 或上传 Code。")
                            .setPositiveButton("选择这个账号", (d, w) -> {
                                if (!connection.equals(AppState.settings.connectionId())) { message("服务器配置已变化，请重新选择账号"); return; }
                                AppState.settings.target(account); navigation.setSelectedItemId(R.id.nav_capture);
                            })
                            .setNegativeButton("取消", null).show());
                    return;
                }
                throw new IllegalStateException("通知对应的账号已不存在，请在服务器确认");
            });
        }
    }
    private void showPage() {
        status = captureSummary = target = logs = monitorStatus = connections = null; vpnButton = monitorButton = qqButton = null;
        content.removeAllViews();
        switch (page) { case 1: home(); break; case 2: accounts(); break; case 3: logs(); break; default: settings(); }
        scroll.scrollTo(0, 0); refresh();
    }
    private void heading(String title, String detail) {
        TextView eyebrow = text("农场助手  /  ANDROID", 11, R.color.farm_muted); eyebrow.setLetterSpacing(.08f); add(content, eyebrow, 4);
        TextView heading = text(title, 28, R.color.farm_ink); heading.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL)); add(content, heading, 4);
        add(content, text(detail, 13, R.color.farm_muted), 20);
    }
    private void home() {
        heading("在手机上续登农场", "获取只在本机进行，推送前由你确认账号。");
        LinearLayout hero = card("QQ 全量接管", "仅 QQ · TCP / UDP / DNS · 最长 2 分钟");
        status = text("", 22, R.color.farm_ink); status.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL)); add(hero, status, 14);
        TextView path = text("QQ 农场   →   本机解析   →   你的服务器", 12, R.color.farm_accent);
        path.setGravity(Gravity.CENTER); path.setPadding(dp(10), dp(14), dp(10), dp(14)); path.setBackground(tint()); add(hero, path, 14);
        vpnButton = button(hero, "启动 VPN 获取", true, this::toggleVpn);
        qqButton = button(hero, "打开 QQ", false, () -> {
            if (AppState.capturing && !AppState.captureReady) { message("VPN 仍在准备，请等待转发就绪"); return; }
            Intent launch = getPackageManager().getLaunchIntentForPackage("com.tencent.mobileqq");
            if (launch == null) message("未找到 QQ，请先安装官方 QQ 客户端"); else startActivity(launch);
        });
        add(hero, text("入口不按 IP / 域名筛选。透传诊断不解密；获取模式仅解析农场目标 TLS。其他应用不接管，现有 VPN 会被替换。", 12, R.color.farm_muted), 0);

        LinearLayout result = card("本次结果", "Code 不落盘、不写日志；仅在内存保留 5 分钟。");
        captureSummary = text("", 15, R.color.farm_ink); add(result, captureSummary, 12);
        target = text("", 13, R.color.farm_accent); add(result, target, 8);
        button(result, "选择服务器账号", false, () -> selectAccounts(false));
        button(result, "新增服务器账号并同步", false, this::createAccount);
        button(result, "检查目标账号登录状态", false, this::verifyTarget);
        button(result, "推送 Code + 好友", true, () -> push(true, true));
        LinearLayout pair = row();
        button(pair, "只推送 Code", false, () -> push(true, false));
        button(pair, "只同步好友", false, () -> push(false, true)); add(result, pair, 4);
        button(result, "手动填写 / 从分享导入", false, () -> manual(""));
        button(result, "清除本次结果", false, AppState::clearCapture);
        LinearLayout notice = card("第一次使用？", "先在设置中填写服务器，再生成并安装证书。系统安装成功不代表 QQ 一定信任。");
        button(notice, "配置服务器地址与 Token", true, this::configureServer).setId(R.id.configure_server);
        button(notice, "查看证书与权限", false, this::permissions);
    }
    private void accounts() {
        heading("账号与离线提醒", "通知独立运行，不需要一直开启 VPN。");
        LinearLayout server = card("连接你的服务器", AppState.settings.server().isEmpty() ? "尚未配置服务器" : AppState.settings.server());
        button(server, "配置服务器地址与 Token", true, this::configureServer).setId(R.id.configure_server);
        button(server, "测试连接", false, () -> { if (!requireServer()) return; async("正在测试连接", () -> {
            String result = new ApiClient(AppState.settings).ping(); runOnUiThread(() -> message(result));
        }); });
        button(server, "新增服务器账号并同步", false, this::createAccount);
        button(server, "选择需要提醒的账号", false, () -> selectAccounts(true));
        LinearLayout monitor = card("账号离线后，在这里接续", "默认每 60 秒检查一次，连续两次成功检查均离线才提醒。断网会重新计数；手机休眠、省电或后台限制会延迟检查，不保证即时送达。手动停止、断线和登录失效都可能触发。");
        monitorStatus = text("", 15, R.color.farm_ink); add(monitor, monitorStatus, 8);
        monitorButton = button(monitor, "开启独立通知", true, () -> {
            if (AppState.monitoring) stopService(new Intent(this, AccountMonitorService.class));
            else if (!requireServer()) return;
            else if (AppState.settings.monitoredIds().isEmpty()) selectAccounts(true);
            else notificationPermission(() -> startForegroundService(new Intent(this, AccountMonitorService.class).setAction("START")));
        });
        button(monitor, "通知权限与电池设置", false, this::permissions);
        LinearLayout how = card("从提醒到重新登录", "收到离线提醒 → 点击通知确认账号 → 在 QQ 切换到该账号 → 获取 Code → 确认并推送。推送绑定服务器账号 ID，不靠备注猜测身份。");
        button(how, "前往获取", false, () -> navigation.setSelectedItemId(R.id.nav_capture));
        LinearLayout upstream = card("关于上游 webhook", "上游支持实例级下线提醒和自定义 webhook。手机通常没有公网入站地址，不能把手机 IP 直接填作 webhook。当前版本采用独立状态检查，无需额外推送服务器。");
        button(upstream, "打开服务器管理页面", false, () -> openWeb(AppState.settings.server()));
    }
    private void logs() {
        heading("运行日志", "最近 200 条 · 仅驻留内存 · 凭据与请求地址脱敏");
        LinearLayout actions = row();
        button(actions, "导出日志", false, () -> export("logs")); button(actions, "清空", false, AppState::clearLogs); add(content, actions, 12);
        LinearLayout metadata = card("本次连接", "只展示 TLS 主机和转发统计，不读取路径、参数或聊天。主机清单仅留内存，不随日志导出。");
        connections = text("", 12, R.color.farm_ink); connections.setTypeface(Typeface.MONOSPACE); connections.setTextIsSelectable(true); add(metadata, connections, 0);
        LinearLayout box = card("运行记录", "停留底部时自动滚动；上滑阅读时不抢位置。");
        logs = text("", 12, R.color.farm_ink); logs.setTypeface(Typeface.MONOSPACE); logs.setTextIsSelectable(true); add(box, logs, 0);
    }
    private void settings() {
        heading("设置与权限", "每一项权限都有用途，所有常驻功能都可停止。");
        LinearLayout server = card("服务器", "Token 用 Android Keystore 加密保存。不会跟随系统备份，也不会发送给更新源。");
        TextInputEditText address = input(server, "同步服务器地址（含协议）", AppState.settings.server(), false);
        address.setId(R.id.server_address); address.setSingleLine(true);
        address.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        address.setImeOptions(EditorInfo.IME_ACTION_NEXT | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        TextInputEditText token = input(server, "服务器管理 Token", "", true);
        token.setId(R.id.server_token); token.setSingleLine(true);
        fieldLayout(address).setHelperText("例如 https://farm.example.com 或 http://IP:端口；不是 webhook 或登录页地址。");
        fieldLayout(token).setHelperText("建议使用服务器的长期 API Token。同一地址留空保留原值；更换地址须重新填写。");
        MaterialSwitch http = toggle(server, "允许此服务器使用 HTTP（明文）", AppState.settings.allowHttp());
        http.setId(R.id.server_http);
        add(server, text("HTTP 会明文传输管理 Token 和 Code；建议给你的 Node 服务配置受信任的 HTTPS。", 12, R.color.farm_muted), 6);
        TextView feedback = text(AppState.settings.hasToken() ? "管理 Token 已保存（不会回显）" : "尚未保存服务器与管理 Token", 13, R.color.farm_accent);
        feedback.setId(R.id.server_feedback); feedback.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE); add(server, feedback, 8);
        button(server, "保存服务器设置", true, () -> {
            String enteredToken = value(token), enteredAddress = value(address); boolean allowHttp = http.isChecked();
            fieldLayout(address).setError(null); fieldLayout(token).setError(null);
            try { UrlPolicy.server(enteredAddress, allowHttp); }
            catch (IllegalArgumentException error) { fieldLayout(address).setError(error.getMessage()); address.requestFocus(); return; }
            if (enteredToken.isEmpty() && (!AppState.settings.hasToken()
                    || !UrlPolicy.server(enteredAddress, allowHttp).toString().equals(AppState.settings.server()))) {
                fieldLayout(token).setError("首次配置或更换地址时必须填写管理 Token"); token.requestFocus(); return;
            }
            async("正在保存", () -> {
                boolean changed = AppState.settings.saveServer(enteredAddress, enteredToken, allowHttp);
                if (changed) { stopService(new Intent(this, AccountMonitorService.class)); AppState.clearCapture(); }
                runOnUiThread(() -> {
                    token.setText(""); dismissKeyboard();
                    feedback.setText(changed ? "已保存；请到账号页测试连接并选择账号" : "已保存；管理 Token 不会回显");
                    message("服务器设置已保存");
                });
            });
        }).setId(R.id.save_server);
        token.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        token.setOnEditorActionListener((v, action, event) -> {
            if (action != EditorInfo.IME_ACTION_DONE) return false;
            server.findViewById(R.id.save_server).performClick(); return true;
        });
        LinearLayout options = card("获取与通知", "获取不会自动上传；始终需要你确认目标账号。");
        MaterialSwitch friends = toggle(options, "同时读取官方好友 GID", AppState.settings.friendCapture());
        MaterialSwitch passthrough = toggle(options, "全量透传诊断（不解密、不获取凭据）", AppState.settings.passthrough());
        passthrough.setId(R.id.passthrough_mode);
        add(options, text("两种模式均只接管 QQ 的全部 TCP/UDP。先用透传诊断确认联网正常，再关闭此项获取农场凭据；无需重启 QQ。", 12, R.color.farm_muted), 6);
        TextInputEditText interval = input(options, "账号检查间隔（30–900 秒）", Integer.toString(AppState.settings.pollSeconds()), false);
        interval.setId(R.id.poll_interval); interval.setSingleLine(true);
        interval.setInputType(InputType.TYPE_CLASS_NUMBER);
        MaterialSwitch accelerated = toggle(options, "使用下载加速", AppState.settings.accelerated());
        TextInputEditText mirror = input(options, "HTTPS 加速地址", AppState.settings.mirror(), false);
        mirror.setId(R.id.download_mirror); mirror.setSingleLine(true);
        button(options, "保存获取与通知设置", false, () -> {
            try {
                int seconds = Integer.parseInt(value(interval));
                if (seconds < 30 || seconds > 900) throw new IllegalArgumentException("检查间隔须在 30–900 秒之间");
                String base = UrlPolicy.server(value(mirror), false).toString();
                AppState.settings.options(friends.isChecked(), accelerated.isChecked(), base, seconds);
                AppState.settings.passthrough(passthrough.isChecked());
                message("已保存；获取设置下次启动 VPN 时生效");
            } catch (Exception e) { message("请检查时间范围与 HTTPS 加速地址"); }
        });
        LinearLayout permissions = card("权限与证书", "不申请 root、无障碍、联系人、短信、定位或全盘文件权限。CA 仅由你手动安装和移除。");
        button(permissions, "管理证书与权限", true, this::permissions);
        LinearLayout updates = card("应用更新", "安卓 v" + BuildConfig.VERSION_NAME + " · 只检查 android-v* 发行版。下载后验证 SHA-256、包名、版本和签名，安装需你确认。");
        button(updates, "检查安卓应用更新", false, this::checkUpdates);
        if (verifiedUpdate != null) button(updates, "安装已验证的更新", true, this::installUpdate);
        button(updates, "查看源代码", false, () -> openWeb("https://github.com/" + UrlPolicy.REPOSITORY));
        button(content, "清除服务器与 Token", false, () -> new MaterialAlertDialogBuilder(this).setTitle("清除服务器设置？")
                .setMessage("将停止账号通知、清除服务器凭据和内存中的获取结果。保留获取、更新及检查间隔设置；不会删除服务器账号或手机系统中已安装的 CA。")
                .setNegativeButton("取消", null).setPositiveButton("清除", (d, w) -> {
                    stopService(new Intent(this, AccountMonitorService.class)); AppState.settings.forgetServer(); AppState.clearCapture(); showPage();
                }).show());
    }
    private void refresh() {
        if (status != null) status.setText(AppState.status);
        if (vpnButton != null) vpnButton.setText(AppState.capturing ? "停止 VPN 获取" : "启动 VPN 获取");
        if (qqButton != null) qqButton.setEnabled(!AppState.capturing || AppState.captureReady);
        if (connections != null) connections.setText(AppState.connectionText());
        if (target != null) target.setText(getString(R.string.target_status, AppState.settings.targetName(), AppState.settings.targetId().isEmpty() ? "" : " · ID " + AppState.settings.targetId()));
        if (captureSummary != null) {
            AppState.Capture result = AppState.capture();
            captureSummary.setText(result == null ? "还没有获取结果\n启动后，打开 QQ 并进入农场。" :
                    (result.code().isEmpty() ? "Code 已清除或已使用" : "Code 已取得（已隐藏）") + "\n好友：" +
                            (result.friendsObserved() ? result.gids().size() + " 位，已排除已知自身 GID" : result.gids().isEmpty() ? "没有待同步好友（未取得或已同步）" : "手动导入 " + result.gids().size() + " 位（未经官方回包验证）") +
                            "\n身份：" + (result.ownGid().isEmpty() ? "未取得自身 GID，请人工核对账号" : "已取得官方自身 GID"));
        }
        if (monitorStatus != null) monitorStatus.setText(getString(R.string.monitor_status, AppState.monitoring ? "正在独立检查" : "通知未运行", AppState.settings.monitoredIds().size())
                + "\n" + AppState.monitorDetail + (AppState.monitorLastCheckedAt == 0 ? "" : "\n上次成功检查：" + java.text.DateFormat.getTimeInstance().format(new java.util.Date(AppState.monitorLastCheckedAt))));
        if (monitorButton != null) monitorButton.setText(AppState.monitoring ? "停止独立通知" : "开启独立通知");
        if (logs != null) {
            boolean bottom = !scroll.canScrollVertically(1);
            String text = AppState.logText(); if (!text.contentEquals(logs.getText())) logs.setText(text.isEmpty() ? "暂无日志" : text);
            if (bottom) scroll.post(() -> scroll.fullScroll(View.FOCUS_DOWN));
        }
    }
    private void toggleVpn() {
        if (AppState.capturing) { stopService(new Intent(this, CaptureVpnService.class)); return; }
        if (!AppState.settings.passthrough() && !new CertificateManager(this).exists()) { permissions(); return; }
        new MaterialAlertDialogBuilder(this).setTitle("启动仅 QQ 的本机 VPN？")
                .setMessage("按 QQ 应用接管全部 TCP/UDP/DNS，不按固定 IP 或域名筛选流量入口，其他应用不接管。"
                        + (AppState.settings.passthrough() ? "本次仅透传，不解密 TLS，不获取 Code 或好友。" : "仅解析农场目标 TLS 的登录和官方好友 GID，其他连接保持透传；QQ 需信任用户 CA。")
                        + "现有其他 VPN 会被替换。最多 2 分钟，可随时停止。无需重启 QQ，启动后重新进入农场即可。")
                .setNegativeButton("取消", null).setPositiveButton("继续授权", (d, w) -> {
                    Intent prepare = VpnService.prepare(this);
                    if (prepare != null) startActivityForResult(prepare, 10); else startCapture();
                }).show();
    }
    private void startCapture() { startForegroundService(new Intent(this, CaptureVpnService.class).setAction("START")); }
    private void configureServer() { dismissKeyboard(); navigation.setSelectedItemId(R.id.nav_settings); }
    private boolean requireServer() {
        if (!AppState.settings.server().isEmpty() && AppState.settings.hasToken()) return true;
        configureServer(); message("请先填写服务器地址和管理 Token，并点击保存"); return false;
    }
    private void dismissKeyboard() {
        getSystemService(InputMethodManager.class).hideSoftInputFromWindow(root.getWindowToken(), 0); root.requestFocus();
    }
    private void selectAccounts(boolean multiple) {
        if (!requireServer()) return;
        async("正在读取服务器账号", () -> {
            ApiClient client = new ApiClient(AppState.settings);
            List<ApiClient.Account> accounts = client.accounts();
            if (!multiple) accounts.removeIf(account -> !account.platform.equals("qq"));
            if (accounts.isEmpty()) {
                runOnUiThread(() -> new MaterialAlertDialogBuilder(this).setTitle("服务器还没有可选账号")
                        .setMessage("可先在手机获取 Code，然后使用「新增服务器账号并同步」。只有好友 GID 时，需要选择已有账号。")
                        .setPositiveButton("新增账号", (d, w) -> createAccount()).setNegativeButton("取消", null).show());
                return;
            }
            runOnUiThread(() -> {
                if (!client.isCurrent() || isFinishing() || isDestroyed()) return;
                String[] labels = accounts.stream().map(ApiClient.Account::label).toArray(String[]::new);
                MaterialAlertDialogBuilder dialog = new MaterialAlertDialogBuilder(this).setTitle(multiple ? "选择需要离线提醒的账号" : "选择本次推送的 QQ 账号");
                if (multiple) {
                    Set<String> selected = AppState.settings.monitoredIds(); boolean[] checked = new boolean[accounts.size()];
                    for (int i = 0; i < checked.length; i++) checked[i] = selected.contains(accounts.get(i).id);
                    dialog.setMultiChoiceItems(labels, checked, (d, index, value) -> { if (value) selected.add(accounts.get(index).id); else selected.remove(accounts.get(index).id); })
                            .setPositiveButton("保存选择", (d, w) -> { if (!client.isCurrent()) { message("服务器配置已变更，请重新选择"); return; } AppState.settings.monitor(selected); if (selected.isEmpty()) stopService(new Intent(this, AccountMonitorService.class)); refresh(); });
                } else dialog.setItems(labels, (d, index) -> { if (!client.isCurrent()) { message("服务器配置已变更，请重新选择"); return; } AppState.settings.target(accounts.get(index)); refresh(); });
                dialog.setNegativeButton("取消", null).show();
            });
        });
    }
    private void createAccount() {
        if (!requireServer()) return;
        AppState.Capture capture = AppState.capture();
        if (capture == null || capture.code().isEmpty()) {
            new MaterialAlertDialogBuilder(this).setTitle("新增账号需要新的 Code")
                    .setMessage("无需预先在服务器创建账号。先在手机获取或手动导入 Code，再点击「新增服务器账号并同步」。仅好友 GID 不能登录新账号。")
                    .setPositiveButton("前往获取", (d, w) -> navigation.setSelectedItemId(R.id.nav_capture))
                    .setNeutralButton("手动导入", (d, w) -> manual(""))
                    .setNegativeButton("取消", null).show(); return;
        }
        String connection = AppState.settings.connectionId();
        String nonce = java.util.UUID.randomUUID().toString().replace("-", "").substring(0, 12);
        LinearLayout form = column(); form.setPadding(dp(24), 0, dp(24), 0);
        TextInputEditText name = input(form, "新账号备注", "", false); name.setSingleLine(true);
        MaterialSwitch friends = toggle(form, "同时增量同步本次好友 GID", !capture.gids().isEmpty());
        friends.setEnabled(!capture.gids().isEmpty());
        add(form, text("上游同名备注会更新旧账号。为区分新增操作，实际备注会附加「 · 手机-" + nonce + "」，提交前会展示完整备注并检查已有账号。不会自动加入离线提醒。", 12, R.color.farm_muted), 8);
        ScrollView scroller = new ScrollView(this); scroller.addView(form);
        var dialog = new MaterialAlertDialogBuilder(this).setTitle("新增服务器账号")
                .setView(scroller).setNegativeButton("取消", null).setPositiveButton("检查并确认", null).create();
        dialog.getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        dialog.setOnShowListener(ignored -> dialog.getButton(-1).setOnClickListener(view -> {
            String baseName, actualName;
            try { baseName = AccountNamePolicy.base(value(name)); actualName = AccountNamePolicy.unique(baseName, nonce); fieldLayout(name).setError(null); }
            catch (IllegalArgumentException error) { fieldLayout(name).setError(error.getMessage()); return; }
            boolean withFriends = friends.isChecked();
            async("正在检查是否重复", () -> {
                if (!connection.equals(AppState.settings.connectionId()) || AppState.capture() != capture)
                    throw new IllegalStateException("服务器或本次结果已变化，请重新确认");
                new ApiClient(AppState.settings).checkNewAccount(baseName, actualName, capture, withFriends);
                runOnUiThread(() -> {
                    if (isFinishing() || isDestroyed() || !dialog.isShowing()) return;
                    dialog.dismiss();
                    new MaterialAlertDialogBuilder(this).setTitle("确认新增，而不是更新已有账号？")
                            .setMessage("实际服务器备注：\n" + actualName + "\n\n将提交本次 Code，由服务器分配新账号 ID 并按其规则自动启动。"
                                    + (withFriends ? "\n拿到新 ID 后增量同步 " + capture.gids().size() + " 个好友 GID。" : "\n本次不提交好友。")
                                    + "\n请确认这是你有权管理的新账号。未取得自身 GID 时无法自动排除同一 QQ 的其他备注。")
                            .setNegativeButton("取消", null).setPositiveButton("确认新增并同步", (d, w) -> async("正在新增账号", () -> {
                                if (!connection.equals(AppState.settings.connectionId()) || AppState.capture() != capture)
                                    throw new IllegalStateException("服务器或本次结果已变化，请重新确认");
                                ApiClient.Account created = new ApiClient(AppState.settings).create(baseName, actualName, capture, withFriends);
                                runOnUiThread(() -> { navigation.setSelectedItemId(R.id.nav_capture); message("新账号已创建，已选择服务器账号 ID：" + created.id); });
                            })).show();
                });
            });
        })); dialog.show();
    }
    private void verifyTarget() {
        if (!requireServer()) return;
        String id = AppState.settings.targetId();
        if (id.isEmpty()) { message("请先选择或新增服务器账号"); return; }
        async("正在检查目标账号是否已登录", () -> {
            ApiClient client = new ApiClient(AppState.settings);
            for (ApiClient.Account account : client.accounts()) if (account.id.equals(id)) {
                boolean connected = client.connected(account);
                if (!client.isCurrent() || !id.equals(AppState.settings.targetId())) throw new IllegalStateException("服务器或目标账号已变化，请重新检查");
                AppState.log("服务器", connected ? "已验证目标账号处于连接状态" : "目标账号尚未连接；提交成功不代表登录已成功，请到服务器管理页检查");
                runOnUiThread(() -> message(connected ? "服务器已确认：目标账号已连接" : "目标账号尚未连接；请稍后再检查或查看服务器日志")); return;
            }
            throw new IllegalStateException("目标账号已不存在，请重新选择");
        });
    }
    private void manual(String initial) {
        LinearLayout form = column(); form.setPadding(dp(24), 0, dp(24), 0);
        TextInputEditText code = input(form, "Code 或完整农场登录 URL（可留空）", initial, true);
        code.setId(R.id.manual_code); code.setSingleLine(true);
        TextInputEditText gids = input(form, "好友 GID（逗号或换行分隔，可留空）", "", false); gids.setMinLines(2);
        gids.setId(R.id.manual_gids); gids.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS); gids.setGravity(Gravity.TOP);
        TextView note = text("仅在内存保留 5 分钟。这里不能验证 QQ 当前身份；推送前请核对服务器账号。", 12, R.color.farm_muted); add(form, note, 0);
        ScrollView formScroll = new ScrollView(this); formScroll.addView(form);
        var dialog = new MaterialAlertDialogBuilder(this).setTitle("手动导入").setView(formScroll).setNegativeButton("取消", null).setPositiveButton("保留到内存", null).create();
        dialog.getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        dialog.setOnShowListener(d -> dialog.getButton(-1).setOnClickListener(v -> {
            try {
                String value = value(code).isEmpty() ? "" : UrlPolicy.manualCode(value(code));
                List<String> friends = UrlPolicy.gids(value(gids), "");
                if (value.isEmpty() && friends.isEmpty()) throw new IllegalArgumentException("请至少填写 Code 或好友 GID");
                AppState.captured(value, "", friends, false); AppState.status = "已手动导入，请核对账号";
                code.setText(""); gids.setText(""); dialog.dismiss(); refresh();
            } catch (Exception e) { message(e.getMessage()); }
        })); dialog.show();
    }
    private void push(boolean code, boolean friends) {
        AppState.Capture capture = AppState.capture();
        if (capture == null) { message("没有结果或结果已清除，请重新获取"); return; }
        if (AppState.settings.targetId().isEmpty()) {
            new MaterialAlertDialogBuilder(this).setTitle("选择同步方式")
                    .setItems(new String[]{"更新已有服务器账号", "新增服务器账号并同步"}, (dialog, which) -> {
                        if (which == 0) selectAccounts(false); else createAccount();
                    }).setNegativeButton("取消", null).show(); return;
        }
        if (code && capture.code().isEmpty()) { message("没有可推送的 Code"); return; }
        if (friends && capture.gids().isEmpty()) { message("没有好友 GID；如只需 Code，请点击「只推送 Code」"); return; }
        String id = AppState.settings.targetId();
        String connection = AppState.settings.connectionId();
        new MaterialAlertDialogBuilder(this).setTitle("确认更新这个服务器账号？")
                .setMessage(AppState.settings.targetName() + "\n服务器账号 ID：" + id + "\n\n" +
                        (code ? "将提交新的 Code 并启动远程账号。" : "不提交 Code，只增量添加好友。") +
                        "\n请确认 QQ 当前登录的正是这个账号。好友不会覆盖已有列表。")
                .setNegativeButton("取消", null).setPositiveButton("确认推送", (d, w) -> async("正在推送", () -> {
                    if (AppState.capture() != capture) throw new IllegalStateException("结果已过期或变更，请重新确认");
                    if (!connection.equals(AppState.settings.connectionId()) || !id.equals(AppState.settings.targetId())) throw new IllegalStateException("服务器或目标账号已变化，请重新确认");
                    new ApiClient(AppState.settings).push(id, capture, code, friends);
                    AppState.log("推送", "所选数据已提交，未提交的数据仍保留至本次结果过期"); runOnUiThread(() -> message("推送完成；未提交的 Code 或好友仍可单独推送"));
                })).show();
    }
    private void permissions() {
        BottomSheetDialog sheet = new BottomSheetDialog(this);
        ScrollView scroller = new ScrollView(this); LinearLayout body = column(); body.setPadding(dp(24), dp(18), dp(24), dp(28)); scroller.addView(body);
        add(body, text("权限与证书", 24, R.color.farm_ink), 10);
        add(body, text("网络：连接你配置的服务器与官方网关。VPN：仅按你点击临时启动。通知：离线提醒与运行状态。安装应用：仅用于你确认的更新。", 13, R.color.farm_muted), 14);
        TextView info = text("正在读取证书状态…", 13, R.color.farm_ink); info.setTypeface(Typeface.MONOSPACE); add(body, info, 12);
        Runnable load = () -> AppState.IO.execute(() -> {
            CertificateManager manager = new CertificateManager(this);
            String result;
            try { result = manager.exists() ? CertificateManager.NAME + "\n有效至：" + manager.certificate().getNotAfter() +
                    "\nSHA-256：\n" + manager.fingerprint() + "\n系统证书库：" + (manager.visibleInSystem() ? "已发现此证书" : "未发现此证书") +
                    "\nQQ 目标握手：" + (AppState.tlsVerified ? "本次已验证" : "尚未验证") : "尚未生成本地证书"; }
            catch (Exception e) { result = "证书读取失败或已过期，请移除后重新生成"; }
            String value = result; runOnUiThread(() -> info.setText(value));
        }); load.run();
        button(body, "生成本地 CA", true, () -> async("正在生成证书", () -> { new CertificateManager(this).generate(); runOnUiThread(load); AppState.log("证书", "已生成本地 CA；私钥保留在 Android Keystore，尚未安装到系统"); }));
        button(body, "导出证书供手动安装", false, () -> export("certificate"));
        add(body, text("Android 11 及以上需到系统「安全 → 加密与凭据 → 安装证书 → CA 证书」手动安装导出的 .cer 文件。安装后通常保留至你移除、清除设备或证书过期，但 QQ 仍可能不信任用户 CA。不会绕过证书固定或请求 root。", 12, R.color.farm_muted), 10);
        button(body, "前往系统安装 / 移除证书", false, () -> openSettings(new Intent(Settings.ACTION_SECURITY_SETTINGS)));
        button(body, "移除助手本地证书与私钥", false, () -> new MaterialAlertDialogBuilder(this).setTitle("移除本地证书？")
                .setMessage("先停止 VPN。将永久删除助手持有的 CA 私钥和本地公钥文件，无法恢复。系统已安装的副本不会随之删除，请另到系统凭据设置移除「" + CertificateManager.NAME + "」。之后重新生成的证书需要重新安装。")
                .setNegativeButton("取消", null).setPositiveButton("移除本地材料", (d, w) -> async("正在移除", () -> {
                    new CertificateManager(this).deleteLocal(); AppState.log("证书", "助手本地证书与私钥已删除，不可恢复；系统副本需手动移除"); runOnUiThread(load);
                })).show());
        button(body, "刷新证书状态", false, load);
        button(body, "允许离线提醒通知", false, () -> notificationPermission(() -> message("通知权限已允许，可在账号页开启独立通知")));
        button(body, "系统通知设置（含弹出提醒）", false, () -> openSettings(new Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, getPackageName())));
        button(body, "电池后台限制设置", false, () -> openSettings(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)));
        add(body, text("不会默认申请忽略省电，不开机自启。休眠和省电会延迟检查，系统也可能终止后台任务；常驻通知消失后需手动重新开启。锁屏通知默认隐藏内容。", 12, R.color.farm_muted), 10);
        button(body, "关闭", false, sheet::dismiss); sheet.setContentView(scroller); sheet.show();
    }
    private void notificationPermission(Runnable action) {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            afterPermission = action; requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 11);
        } else if (!NotificationManagerCompat.from(this).areNotificationsEnabled()) {
            message("系统通知已关闭，请在「系统通知设置」中开启；不会启动后台检查");
            permissions();
        } else action.run();
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == 11) { Runnable action = afterPermission; afterPermission = null;
            if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED && action != null) action.run();
            else message("未允许通知，请在系统通知设置中开启；不会启动后台检查"); }
    }
    private void export(String type) {
        if (type.equals("certificate") && !new CertificateManager(this).exists()) { message("请先生成本地证书"); return; }
        pendingExport = type;
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(type.equals("certificate") ? "application/pkix-cert" : "text/plain")
                .putExtra(Intent.EXTRA_TITLE, type.equals("certificate") ? "FarmHelper-LocalCA.cer" : "farm-helper-redacted-log.txt");
        startActivityForResult(intent, 12);
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == 10) { if (result == Activity.RESULT_OK) startCapture(); else message("VPN 未授权，网络未被接管"); }
        if (request == 12 && result == Activity.RESULT_OK && data != null && data.getData() != null && pendingExport != null) {
            String type = pendingExport; pendingExport = null; Uri uri = data.getData();
            async("正在导出", () -> {
                byte[] bytes = type.equals("certificate") ? new CertificateManager(this).export() : AppState.logText().getBytes(java.nio.charset.StandardCharsets.UTF_8);
                try (var output = getContentResolver().openOutputStream(uri, "wt")) { if (output == null) throw new IllegalStateException("无法写入目标文件"); output.write(bytes); }
                runOnUiThread(() -> message(type.equals("certificate") ? "公钥证书已导出，尚未安装；请到系统安全设置手动安装" : "脱敏日志已导出"));
            });
        }
    }
    private void checkUpdates() {
        async("正在检查安卓发行版", () -> {
            UpdateManager.Release release = UpdateManager.check(AppState.settings);
            runOnUiThread(() -> {
                if (release == null) { message("尚无包含校验信息的安卓稳定版；不会误装 Windows 更新"); return; }
                if (UpdateManager.compare(release.version(), BuildConfig.VERSION_NAME) <= 0) { message("当前安卓应用已是最新稳定版"); return; }
                new MaterialAlertDialogBuilder(this).setTitle("安卓 v" + release.version()).setMessage(release.notes().isEmpty() ? "发现新版本。下载后校验并由系统确认安装。" : release.notes())
                        .setNegativeButton("稍后", null).setPositiveButton("下载并校验", (d, w) -> async("正在下载并校验更新", () -> {
                            File file = UpdateManager.download(this, AppState.settings, release); verifiedUpdate = file;
                            runOnUiThread(() -> new MaterialAlertDialogBuilder(this).setTitle("更新包验证通过").setMessage("SHA-256、包名和签名已匹配。现在打开系统安装器？")
                                    .setNegativeButton("稍后", (dialog, which) -> showPage()).setPositiveButton("确认安装", (dialog, which) -> installUpdate()).show());
                        })).show();
            });
        });
    }
    private void installUpdate() {
        if (verifiedUpdate == null || !verifiedUpdate.isFile()) { message("请先下载并验证更新"); return; }
        if (!getPackageManager().canRequestPackageInstalls()) {
            new MaterialAlertDialogBuilder(this).setTitle("允许安装此更新？").setMessage("系统要求你单独允许此应用安装更新。授权后请返回设置页，点击「安装已验证的更新」。")
                    .setNegativeButton("取消", null).setPositiveButton("前往系统授权", (d, w) -> {
                        openSettings(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + getPackageName()))); showPage();
                    }).show(); return;
        }
        Uri uri = FileProvider.getUriForFile(this, getPackageName() + ".files", verifiedUpdate);
        startActivity(new Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION));
    }
    private interface Work { void run() throws Exception; }
    private void async(String label, Work work) {
        if (busy) { message("上一项操作尚未完成"); return; } busy = true; message(label);
        AppState.IO.execute(() -> {
            try { work.run(); }
            catch (Exception error) {
                String detail = error instanceof java.net.ConnectException || error instanceof java.net.UnknownHostException ? "无法连接，请检查服务器地址与网络" :
                        error instanceof org.json.JSONException ? "服务器响应格式不正确" : error.getMessage();
                if (detail == null || detail.contains("://") || detail.length() > 240) detail = "操作失败，请检查配置与网络后重试";
                String safe = detail; AppState.log("操作", "操作未完成；详细提示仅在应用内显示"); runOnUiThread(() -> message(safe));
            } finally { runOnUiThread(() -> { busy = false; refresh(); }); }
        });
    }
    private LinearLayout card(String title, String description) {
        MaterialCardView card = new MaterialCardView(this); card.setRadius(dp(22)); card.setCardElevation(0);
        card.setCardBackgroundColor(getColor(R.color.farm_card)); card.setStrokeWidth(0);
        LinearLayout body = column(); body.setPadding(dp(18), dp(18), dp(18), dp(18)); card.addView(body);
        TextView heading = text(title, 17, R.color.farm_ink); heading.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL)); add(body, heading, 6);
        if (!description.isEmpty()) add(body, text(description, 12, R.color.farm_muted), 12);
        add(content, card, 14); return body;
    }
    private TextInputEditText input(LinearLayout parent, String label, String value, boolean secret) {
        TextInputLayout layout = new TextInputLayout(this); layout.setHint(label); layout.setBoxBackgroundMode(TextInputLayout.BOX_BACKGROUND_OUTLINE);
        if (secret) layout.setEndIconMode(TextInputLayout.END_ICON_PASSWORD_TOGGLE);
        TextInputEditText edit = new TextInputEditText(this); edit.setTextSize(14); edit.setText(value); edit.setSaveEnabled(false);
        edit.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
        edit.setImeOptions(EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        edit.setInputType(InputType.TYPE_CLASS_TEXT | (secret ? InputType.TYPE_TEXT_VARIATION_PASSWORD : InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS));
        layout.addView(edit, new LinearLayout.LayoutParams(-1, -2)); add(parent, layout, 12); return edit;
    }
    private TextInputLayout fieldLayout(TextInputEditText edit) { return (TextInputLayout) edit.getParent().getParent(); }
    private MaterialSwitch toggle(LinearLayout parent, String label, boolean checked) {
        MaterialSwitch view = new MaterialSwitch(this); view.setText(label); view.setTextSize(14); view.setChecked(checked); view.setMinHeight(dp(48)); add(parent, view, 8); return view;
    }
    private MaterialButton button(LinearLayout parent, String label, boolean filled, Runnable action) {
        MaterialButton view = new MaterialButton(this, null, filled ? com.google.android.material.R.attr.materialButtonStyle : com.google.android.material.R.attr.materialButtonOutlinedStyle);
        view.setText(label); view.setTextSize(13); view.setAllCaps(false); view.setCornerRadius(dp(14)); view.setMinHeight(dp(48));
        view.setOnClickListener(v -> action.run());
        LinearLayout.LayoutParams params = parent.getOrientation() == LinearLayout.HORIZONTAL ? new LinearLayout.LayoutParams(0, -2, 1) : new LinearLayout.LayoutParams(-1, -2);
        params.bottomMargin = dp(6); if (parent.getOrientation() == LinearLayout.HORIZONTAL) params.setMarginEnd(dp(6));
        parent.addView(view, params); return view;
    }
    private TextView text(String value, int size, int color) { TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(getColor(color)); view.setLineSpacing(dp(3), 1); return view; }
    private LinearLayout column() { LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL); return layout; }
    private LinearLayout row() { LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.HORIZONTAL); return layout; }
    private void add(LinearLayout parent, View view, int bottom) { LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.bottomMargin = dp(bottom); parent.addView(view, params); }
    private android.graphics.drawable.GradientDrawable tint() { var shape = new android.graphics.drawable.GradientDrawable(); shape.setColor(getColor(R.color.farm_tint)); shape.setCornerRadius(dp(12)); return shape; }
    private int dp(int value) { return Math.round(getResources().getDisplayMetrics().density * value); }
    @androidx.annotation.IdRes private static int navId(int page) {
        switch (page) { case 2: return R.id.nav_accounts; case 3: return R.id.nav_logs; case 4: return R.id.nav_settings; default: return R.id.nav_capture; }
    }
    private static String value(TextInputEditText edit) { return edit.getText() == null ? "" : edit.getText().toString().trim(); }
    private void message(String text) { if (!isFinishing() && !isDestroyed()) Toast.makeText(this, text, Toast.LENGTH_LONG).show(); }
    private void openSettings(Intent intent) { try { startActivity(intent); } catch (Exception e) { message("系统未提供此入口，请到系统设置中的应用管理或安全设置操作"); } }
    private void openWeb(String address) {
        try { Uri uri = Uri.parse(UrlPolicy.server(address, AppState.settings.allowHttp()).toString()); startActivity(new Intent(Intent.ACTION_VIEW, uri)); }
        catch (Exception e) { message("地址无效或没有可用浏览器"); }
    }
}
