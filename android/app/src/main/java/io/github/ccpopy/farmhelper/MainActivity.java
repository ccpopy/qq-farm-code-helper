package io.github.ccpopy.farmhelper;

import android.Manifest;
import android.animation.ObjectAnimator;
import android.animation.PropertyValuesHolder;
import android.app.Activity;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.content.pm.PackageManager;
import android.graphics.Rect;
import android.graphics.Typeface;
import android.net.ConnectivityManager;
import android.net.NetworkCapabilities;
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
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import androidx.annotation.Nullable;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.app.NotificationManagerCompat;
import androidx.core.content.FileProvider;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import com.google.android.material.bottomnavigation.BottomNavigationView;
import com.google.android.material.bottomsheet.BottomSheetDialog;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.dialog.MaterialAlertDialogBuilder;
import com.google.android.material.materialswitch.MaterialSwitch;
import com.google.android.material.textfield.TextInputEditText;
import java.io.File;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

public final class MainActivity extends AppCompatActivity {
    private Ui ui;
    private LinearLayout root, content;
    private ScrollView scroll;
    private BottomNavigationView navigation;
    private int page = 1;
    private final Runnable listener = this::refresh;
    private Runnable afterPermission;
    private String pendingExport;
    private File verifiedUpdate;
    private boolean busy;
    private boolean requestingOverlay;
    private MaterialSwitch overlaySwitch;
    // Capture page
    private FrameLayout core;
    private View halo;
    private ImageView coreIcon;
    private TextView ringLabel, ringStatus;
    private MaterialButton appButton;
    private LinearLayout segment, resultSlot;
    private String resultKey;
    private ObjectAnimator breathing;
    private boolean wechatVpn;
    private long handledCapture = -1;
    // Accounts page
    private LinearLayout accountList;
    private MaterialSwitch monitorSwitch;
    private TextView monitorStatus, accountTitle;
    private MaterialButton selectAll;
    private static List<ApiClient.Account> accountCache = new ArrayList<>();
    private static final Map<String, Boolean> onlineCache = new HashMap<>();
    private static String accountCacheConnection = "";
    private static int accountLoad;
    private boolean accountsLoading;
    // Logs page
    private TextView logs, connections;

    @Override protected void onCreate(@Nullable Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ui = new Ui(this);
        Ui.secure(getWindow());
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        root = ui.column(); root.setBackgroundColor(getColor(R.color.farm_surface));
        root.setId(R.id.screen_root); root.setFocusableInTouchMode(true);
        ViewCompat.setOnApplyWindowInsetsListener(root, (view, insets) -> {
            Insets bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
            Insets keyboard = insets.getInsets(WindowInsetsCompat.Type.ime());
            boolean editing = insets.isVisible(WindowInsetsCompat.Type.ime());
            view.setPadding(bars.left, bars.top, bars.right, editing ? keyboard.bottom : 0);
            navigation.setPadding(0, 0, 0, bars.bottom);
            navigation.setVisibility(editing ? View.GONE : View.VISIBLE);
            if (editing) scroll.post(() -> {
                View focus = getCurrentFocus();
                if (focus != null && focus != root) focus.requestRectangleOnScreen(new Rect(0, 0, focus.getWidth(), focus.getHeight() + ui.dp(12)), false);
            });
            // Root owns edge/IME padding; do not let Material navigation add it a second time.
            return WindowInsetsCompat.CONSUMED;
        });
        scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setClipToPadding(false);
        scroll.setId(R.id.screen_scroll);
        content = ui.column(); content.setPadding(ui.dp(20), ui.dp(8), ui.dp(20), ui.dp(28)); scroll.addView(content);
        root.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1));
        View line = new View(this); line.setBackgroundColor(getColor(R.color.farm_line));
        root.addView(line, new LinearLayout.LayoutParams(-1, Math.max(1, ui.dp(.6f))));
        navigation = new BottomNavigationView(this); navigation.setBackgroundColor(getColor(R.color.farm_card));
        navigation.setId(R.id.bottom_navigation); navigation.setElevation(0);
        navigation.setLabelVisibilityMode(BottomNavigationView.LABEL_VISIBILITY_LABELED);
        int[][] states = {{android.R.attr.state_checked}, {}};
        ColorStateList tint = new ColorStateList(states, new int[]{getColor(R.color.farm_accent), getColor(R.color.farm_muted)});
        navigation.setItemIconTintList(tint); navigation.setItemTextColor(tint);
        navigation.setItemActiveIndicatorColor(ColorStateList.valueOf(getColor(R.color.farm_tint)));
        navigation.setItemIconSize(ui.dp(24));
        navigation.getMenu().add(0, R.id.nav_capture, 0, "获取").setIcon(R.drawable.ic_capture);
        navigation.getMenu().add(0, R.id.nav_accounts, 1, "账号").setIcon(R.drawable.ic_accounts);
        navigation.getMenu().add(0, R.id.nav_logs, 2, "日志").setIcon(R.drawable.ic_logs);
        navigation.getMenu().add(0, R.id.nav_settings, 3, "设置").setIcon(R.drawable.ic_settings);
        navigation.setOnItemSelectedListener(item -> { page = item.getOrder() + 1; showPage(); return true; });
        root.addView(navigation, new LinearLayout.LayoutParams(-1, -2)); setContentView(root);
        ViewCompat.requestApplyInsets(root);
        if (savedInstanceState != null) {
            page = savedInstanceState.getInt("page", 1); wechatVpn = savedInstanceState.getBoolean("wechatVpn");
            handledCapture = savedInstanceState.getLong("handledCapture", -1);
            requestingOverlay = savedInstanceState.getBoolean("requestingOverlay");
        }
        navigation.setSelectedItemId(navId(page)); showPage();
        handleIntent(getIntent());
    }
    @Override protected void onResume() {
        super.onResume(); Ui.secure(getWindow()); AppState.listen(listener); syncMonitor(false); refresh();
        if (requestingOverlay) {
            requestingOverlay = false;
            AppState.settings.overlayEnabled(Settings.canDrawOverlays(this));
            if (!Settings.canDrawOverlays(this)) message("未允许悬浮窗，可继续在应用内查看结果");
        }
        if (overlaySwitch != null) Ui.setQuietly(overlaySwitch, AppState.settings.overlayEnabled() && Settings.canDrawOverlays(this));
        AppState.changed();
        if (page == 2) loadAccounts();
    }
    @Override protected void onPause() { AppState.unlisten(listener); super.onPause(); }
    @Override protected void onDestroy() { if (breathing != null) breathing.cancel(); super.onDestroy(); }
    @Override protected void onSaveInstanceState(Bundle state) {
        state.putInt("page", page); state.putBoolean("wechatVpn", wechatVpn); state.putLong("handledCapture", handledCapture);
        state.putBoolean("requestingOverlay", requestingOverlay);
        super.onSaveInstanceState(state);
    }
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
        String accountUuid = intent.getStringExtra("accountUuid");
        intent.removeExtra("accountUuid");
        intent.removeExtra("accountId"); intent.removeExtra("server"); intent.removeExtra("connectionId");
        if (id == null || id.isEmpty()) return;
        if (!AppState.settings.server().equals(server) || connection == null || connection.isEmpty() || !connection.equals(AppState.settings.connectionId())) {
            message("服务器配置已变化，这条通知已失效"); return;
        }
        async("正在打开账号", () -> {
            for (ApiClient.Account account : new ApiClient(AppState.settings).accounts()) if (account.id.equals(id)) {
                if (accountUuid != null && !accountUuid.isEmpty() && !accountUuid.equals(account.uuid))
                    throw new IllegalStateException("原账号已被删除或替换，通知已失效");
                runOnUiThread(() -> relogin(account));
                return;
            }
            throw new IllegalStateException("通知对应的账号已不存在");
        });
    }
    private void showPage() {
        if (breathing != null) { breathing.cancel(); breathing = null; }
        core = null; halo = null; coreIcon = null; ringLabel = ringStatus = null; appButton = null; segment = resultSlot = null; resultKey = null;
        accountList = null; monitorSwitch = overlaySwitch = null; monitorStatus = accountTitle = null; selectAll = null; logs = connections = null;
        content.removeAllViews();
        switch (page) { case 1: capturePage(); break; case 2: accountsPage(); break; case 3: logsPage(); break; default: settingsPage(); }
        scroll.scrollTo(0, 0); refresh();
    }
    private LinearLayout header(String title) {
        LinearLayout bar = ui.row(); bar.setMinimumHeight(ui.dp(64));
        TextView heading = ui.strong(title, 26); heading.setPadding(ui.dp(4), 0, 0, 0);
        heading.setAccessibilityHeading(true);
        bar.addView(heading, new LinearLayout.LayoutParams(0, -2, 1));
        ui.add(content, bar, 4); return bar;
    }

    // ---------------------------------------------------------------- 获取
    private void capturePage() {
        CapturePlatform platform = AppState.settings.capturePlatform();
        boolean scan = platform == CapturePlatform.WECHAT && !wechatVpn;
        header("农场助手");
        segment = segmented(new String[]{"QQ", "微信"}, platform.ordinal(), which -> choosePlatform(CapturePlatform.values()[which]));
        ui.add(content, segment, 10);
        if (AppState.settings.server().isEmpty() || !AppState.settings.hasToken())
            banner("还没有连接服务器", "去设置", this::configureServer);
        else if (!scan && !AppState.settings.passthrough() && !new CertificateManager(this).exists())
            banner("首次使用需要安装证书", "安装", this::certificateSheet);

        // The hero: one soft panel holding the single action of this screen.
        LinearLayout hero = ui.column(); hero.setGravity(Gravity.CENTER_HORIZONTAL);
        android.graphics.drawable.GradientDrawable wash = new android.graphics.drawable.GradientDrawable(
                android.graphics.drawable.GradientDrawable.Orientation.TOP_BOTTOM,
                new int[]{getColor(R.color.farm_hero), getColor(R.color.farm_card)});
        wash.setCornerRadius(ui.dp(28)); hero.setBackground(wash);
        hero.setPadding(ui.dp(20), ui.dp(16), ui.dp(20), ui.dp(24));
        if (platform == CapturePlatform.WECHAT) {
            LinearLayout mode = segmented(new String[]{"扫码登录", "VPN 获取"}, wechatVpn ? 1 : 0, which -> {
                if (AppState.capturing) { message("请先停止获取"); return; }
                if (wechatVpn != (which == 1)) { wechatVpn = which == 1; showPage(); }
            });
            ((LinearLayout.LayoutParams) mode.getLayoutParams()).width = ui.dp(220);
            mode.setBackground(ui.shape(R.color.farm_card, 14));
            hero.addView(mode);
        }
        FrameLayout stage = new FrameLayout(this);
        halo = new View(this); halo.setBackground(ui.oval(R.color.farm_tint));
        stage.addView(halo, new FrameLayout.LayoutParams(ui.dp(176), ui.dp(176), Gravity.CENTER));
        core = new FrameLayout(this); core.setId(scan ? R.id.wx_login : R.id.capture_toggle);
        core.setElevation(ui.dp(3)); core.setClipToOutline(true); ui.ripple(core);
        coreIcon = new ImageView(this);
        core.addView(coreIcon, new FrameLayout.LayoutParams(ui.dp(48), ui.dp(48), Gravity.CENTER));
        core.setOnClickListener(v -> { if (scan) openWxLogin(); else toggleVpn(); });
        stage.addView(core, new FrameLayout.LayoutParams(ui.dp(128), ui.dp(128), Gravity.CENTER));
        LinearLayout.LayoutParams stageParams = new LinearLayout.LayoutParams(-1, ui.dp(200)); stageParams.topMargin = ui.dp(8);
        hero.addView(stage, stageParams);
        ringLabel = ui.strong("", 20); ringLabel.setGravity(Gravity.CENTER); ui.add(hero, ringLabel, 4);
        ringStatus = ui.text("", 13, R.color.farm_muted); ringStatus.setGravity(Gravity.CENTER);
        ringStatus.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE); ui.add(hero, ringStatus, 0);
        ui.add(content, hero, 12);

        LinearLayout actions = ui.row();
        if (!scan) appButton = ui.button(actions, "打开 " + platform.label, Ui.Tone.TONAL, R.drawable.ic_open, () -> {
            if (AppState.capturing && !AppState.captureReady) { message("VPN 正在准备，请稍候"); return; }
            Intent launch = getPackageManager().getLaunchIntentForPackage(platform.packageName);
            if (launch == null) message("未安装 " + platform.label); else startActivity(launch);
        });
        ui.button(actions, "手动导入", Ui.Tone.TONAL, R.drawable.ic_edit, () -> manual(""));
        for (int i = 0; i < actions.getChildCount(); i++) ((MaterialButton) actions.getChildAt(i)).setMinHeight(ui.dp(48));
        ui.add(content, actions, 12);
        resultSlot = ui.column(); ui.add(content, resultSlot, 0);
    }
    private interface Choice { void pick(int which); }
    /** Pill segmented control; the selected option sits on a raised card. */
    private LinearLayout segmented(String[] labels, int selected, Choice choice) {
        LinearLayout group = ui.row(); group.setBackground(ui.shape(R.color.farm_line, 16));
        group.setPadding(ui.dp(4), ui.dp(4), ui.dp(4), ui.dp(4));
        for (int i = 0; i < labels.length; i++) {
            int which = i; boolean on = i == selected;
            TextView tab = ui.text(labels[i], 15, on ? R.color.farm_ink : R.color.farm_muted); tab.setGravity(Gravity.CENTER);
            tab.setMinHeight(ui.dp(40)); tab.setSelected(on);
            tab.setTypeface(on ? Ui.MEDIUM : Typeface.DEFAULT);
            if (on) { tab.setBackground(ui.shape(R.color.farm_card, 12)); tab.setElevation(ui.dp(1)); }
            ui.ripple(tab); tab.setOnClickListener(v -> choice.pick(which));
            group.addView(tab, new LinearLayout.LayoutParams(0, -2, 1));
        }
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.gravity = Gravity.CENTER_HORIZONTAL;
        group.setLayoutParams(params); return group;
    }
    private void banner(String title, String action, Runnable click) {
        LinearLayout box = ui.row(); box.setBackground(ui.shape(R.color.farm_amber_tint, 16));
        box.setPadding(ui.dp(16), ui.dp(8), ui.dp(8), ui.dp(8));
        TextView label = ui.text(title, 14, R.color.farm_amber); label.setTypeface(Ui.MEDIUM);
        box.addView(label, new LinearLayout.LayoutParams(0, -2, 1));
        MaterialButton go = ui.button(box, action, Ui.Tone.TEXT, click);
        go.setBackgroundTintList(ColorStateList.valueOf(getColor(R.color.farm_card))); go.setTextColor(getColor(R.color.farm_amber));
        ui.add(content, box, 10);
    }
    private void refreshCapture() {
        CapturePlatform platform = AppState.settings.capturePlatform();
        boolean scan = platform == CapturePlatform.WECHAT && !wechatVpn;
        boolean running = AppState.capturing && !scan;
        AppState.Capture result = AppState.capture();
        segment.setAlpha(AppState.capturing ? .6f : 1f);
        boolean done = !running && result != null && !scan;
        core.setBackground(ui.oval(running || done ? R.color.farm_accent : R.color.farm_card));
        coreIcon.setImageResource(scan ? R.drawable.ic_qr : done ? R.drawable.ic_check : R.drawable.ic_power);
        coreIcon.setImageTintList(ColorStateList.valueOf(getColor(running || done ? R.color.farm_on_accent : R.color.farm_accent)));
        String label = scan ? "扫码登录" : running ? "停止 VPN" : AppState.settings.passthrough() ? "开始透传诊断" : "开始获取";
        ringLabel.setText(label); core.setContentDescription(label);
        ringStatus.setText(scan ? "通过服务器生成登录二维码" : AppState.capturing || result != null ? AppState.status
                : AppState.settings.passthrough() ? "只测试联网，不获取 Code" : "点击后进入 " + platform.label + " 农场");
        if (appButton != null) appButton.setEnabled(!AppState.capturing || AppState.captureReady);
        if (running && breathing == null) {
            breathing = ObjectAnimator.ofPropertyValuesHolder(halo, PropertyValuesHolder.ofFloat(View.SCALE_X, .82f, 1.06f),
                    PropertyValuesHolder.ofFloat(View.SCALE_Y, .82f, 1.06f), PropertyValuesHolder.ofFloat(View.ALPHA, 1f, .5f));
            breathing.setDuration(1300); breathing.setRepeatCount(ObjectAnimator.INFINITE); breathing.setRepeatMode(ObjectAnimator.REVERSE);
            breathing.start();
        } else if (!running && breathing != null) {
            breathing.cancel(); breathing = null; halo.setScaleX(1); halo.setScaleY(1); halo.setAlpha(1);
        }
        String key = result == null ? "" : result.generation() + "/" + result.code().isEmpty() + "/" + result.gids().size() + "/" + result.ownGid().isEmpty() + "/" + AppState.capturing + "/" + ApiClient.isPushing();
        if (!key.equals(resultKey)) { resultKey = key; renderResult(result); }
    }
    private void renderResult(AppState.Capture result) {
        resultSlot.removeAllViews();
        if (result == null) return;
        LinearLayout body = ui.padded(ui.card(resultSlot));
        LinearLayout title = ui.row();
        title.addView(ui.strong("本次结果", 16), new LinearLayout.LayoutParams(0, -2, 1));
        TextView chip = ui.text(result.platform().label, 12, R.color.farm_accent); chip.setTypeface(Ui.MEDIUM);
        chip.setBackground(ui.shape(R.color.farm_tint, 10)); chip.setPadding(ui.dp(10), ui.dp(3), ui.dp(10), ui.dp(3));
        title.addView(chip);
        ui.add(body, title, 12);
        LinearLayout facts = ui.row();
        fact(facts, result.code().isEmpty() ? "—" : "已获取", "Code");
        fact(facts, Integer.toString(result.gids().size()), "待同步好友");
        fact(facts, result.ownGid().isEmpty() ? "—" : "已识别", "身份");
        ui.add(body, facts, 14);
        LinearLayout actions = ui.row();
        MaterialButton push = ui.button(actions, "推送到服务器", Ui.Tone.PRIMARY, R.drawable.ic_upload, () -> syncCapture(false));
        push.setEnabled(!ApiClient.isPushing() && (!result.code().isEmpty() || !result.gids().isEmpty()));
        MaterialButton clear = ui.button(actions, "清除", Ui.Tone.DANGER, () -> {
            if (ApiClient.isPushing()) { message("正在推送，请等待本次完成"); return; }
            AppState.clearCapture();
        });
        clear.setEnabled(!ApiClient.isPushing());
        clear.setMinHeight(ui.dp(52));
        ui.add(body, actions, 0);
    }
    private void fact(LinearLayout parent, String value, String label) {
        LinearLayout cell = ui.column(); cell.setBackground(ui.shape(R.color.farm_surface, 14));
        cell.setPadding(ui.dp(12), ui.dp(10), ui.dp(12), ui.dp(10));
        cell.addView(ui.strong(value, 17)); cell.addView(ui.text(label, 12, R.color.farm_muted));
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, -2, 1);
        if (parent.getChildCount() > 0) params.setMarginStart(ui.dp(8));
        parent.addView(cell, params);
    }
    private void choosePlatform(CapturePlatform platform) {
        if (platform == AppState.settings.capturePlatform()) return;
        if (AppState.capturing || busy) { message("请先停止获取，再切换应用"); return; }
        if (switchPlatform(platform)) showPage();
    }
    private boolean switchPlatform(CapturePlatform platform) {
        if (AppState.settings.capturePlatform() == platform) return true;
        if (AppState.capturing) { message("请先停止获取，再切换应用"); return false; }
        AppState.settings.capturePlatform(platform);
        AppState.clearCapture(); AppState.tlsVerified = false;
        AppState.status = "准备就绪";
        AppState.log("获取", "已切换到 " + platform.label + "；旧结果与目标账号已清除");
        return true;
    }
    private void openWxLogin() {
        if (!requireServer()) return;
        if (AppState.capturing) { message("请先停止 VPN 获取"); return; }
        startActivity(new Intent(this, WxLoginActivity.class));
    }
    private void toggleVpn() {
        if (AppState.capturing) { startService(new Intent(this, CaptureVpnService.class).setAction("STOP")); return; }
        if (!AppState.settings.passthrough() && !new CertificateManager(this).exists()) { certificateSheet(); return; }
        String hint = networkCompatibilityHint();
        if (!hint.isEmpty()) {
            new MaterialAlertDialogBuilder(this).setTitle("WLAN 与移动数据同时开启")
                    .setMessage(hint).setNegativeButton("取消", null).setPositiveButton("继续", (d, w) -> prepareVpn()).show();
        } else prepareVpn();
    }
    private void prepareVpn() {
        Intent prepare = VpnService.prepare(this);
        if (prepare != null) startActivityForResult(prepare, 10); else startCapture();
    }
    private String networkCompatibilityHint() {
        ConnectivityManager connectivity = getSystemService(ConnectivityManager.class);
        if (connectivity == null) return "";
        NetworkCapabilities active = connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
        if (active == null || !active.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) return "";
        for (android.net.Network network : connectivity.getAllNetworks()) {
            NetworkCapabilities capabilities = connectivity.getNetworkCapabilities(network);
            if (capabilities != null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
                    && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                    && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
                    && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_SUSPENDED)) {
                return "若进入农场超时，请关闭移动数据或系统的“网络加速 / 双通道加速”后重试。";
            }
        }
        return "";
    }
    private void startCapture() { startForegroundService(new Intent(this, CaptureVpnService.class).setAction("START")); }

    // ---------------------------------------------------------------- 推送
    /** Match the capture with a server account; the user always confirms before anything is written. */
    private void syncCapture(boolean automatic) {
        AppState.Capture capture = AppState.capture();
        if (capture == null) { if (!automatic) message("结果已过期，请重新获取"); return; }
        boolean code = AppState.settings.pushCode() && !capture.code().isEmpty();
        boolean friends = AppState.settings.pushFriends() && !capture.gids().isEmpty();
        if (!code && !friends) {
            if (!automatic) message(capture.code().isEmpty() && capture.gids().isEmpty() ? "没有可推送的数据" : "推送内容已在设置中关闭");
            return;
        }
        if (AppState.settings.server().isEmpty() || !AppState.settings.hasToken()) {
            if (automatic) message("连接服务器后即可推送"); else requireServer();
            return;
        }
        async("正在比对服务器账号", () -> {
            ApiClient client = new ApiClient(AppState.settings);
            List<ApiClient.Account> accounts = client.accounts();
            accounts.removeIf(account -> !account.platform.equals(capture.platform().id));
            String own = capture.ownGid(), previousId = AppState.settings.targetId();
            ApiClient.Account byGid = client.matchingAccount(accounts, capture), previous = null;
            for (ApiClient.Account account : accounts) {
                String known = client.knownGid(account);
                if (account.id.equals(previousId) && (own.isEmpty() || known.isEmpty() || own.equals(known))) previous = account;
            }
            ApiClient.Account detected = byGid, last = previous;
            runOnUiThread(() -> {
                AppState.Capture current = AppState.capture();
                if (isFinishing() || isDestroyed() || !client.isCurrent() || current == null || current.generation() != capture.generation()) return;
                if (detected != null) confirmPush(detected, true, accounts, capture, code, friends);
                else if (last != null) confirmPush(last, false, accounts, capture, code, friends);
                else chooseDestination(accounts, capture, code, friends);
            });
        });
    }
    private String pushSummary(AppState.Capture capture, boolean code, boolean friends) {
        int count = capture == null ? 0 : capture.gids().size();
        return code && friends ? "将推送 Code 和 " + count + " 位好友。" : code ? "将推送 Code。" : "将追加 " + count + " 位好友。";
    }
    private void confirmPush(ApiClient.Account account, boolean detected, List<ApiClient.Account> accounts, AppState.Capture capture, boolean code, boolean friends) {
        new MaterialAlertDialogBuilder(this).setTitle(detected ? "检测到已有账号" : "推送到上次的账号？")
                .setMessage(account.name + "\n" + account.identityLabel() + "\n\n" + pushSummary(capture, code, friends))
                .setPositiveButton(detected ? "更新" : "推送", (d, w) -> push(account, capture, code, friends))
                .setNeutralButton("选择其他", (d, w) -> chooseDestination(accounts, capture, code, friends))
                .setNegativeButton("取消", null).show();
    }
    private void chooseDestination(List<ApiClient.Account> accounts, AppState.Capture capture, boolean code, boolean friends) {
        if (!code && accounts.isEmpty()) { message("服务器上没有同平台账号；只有好友数据时无法新增"); return; }
        List<String> labels = new ArrayList<>();
        if (code) labels.add("＋ 新增账号");
        for (ApiClient.Account account : accounts) labels.add(account.selectionLabel());
        new MaterialAlertDialogBuilder(this).setTitle(code ? "选择账号" : "追加好友到")
                .setItems(labels.toArray(new String[0]), (d, which) -> {
                    if (code && which == 0) { createAccount(capture, friends); return; }
                    ApiClient.Account account = accounts.get(which - (code ? 1 : 0));
                    if (!code) { push(account, capture, false, friends); return; }
                    new MaterialAlertDialogBuilder(this).setTitle("覆盖「" + account.name + "」？")
                            .setMessage(account.identityLabel() + "\n这个账号的登录将被替换为本次获取的 Code。\n" + pushSummary(capture, code, friends))
                            .setPositiveButton("覆盖", (x, y) -> push(account, capture, code, friends))
                            .setNegativeButton("取消", null).show();
                }).setNegativeButton("取消", null).show();
    }
    private void push(ApiClient.Account account, AppState.Capture capture, boolean code, boolean friends) {
        if (capture == null) { message("结果已过期，请重新获取"); return; }
        long generation = capture.generation();
        async("正在推送", () -> {
            AppState.Capture current = AppState.capture();
            if (current == null || current.generation() != generation) throw new IllegalStateException("结果已过期，请重新获取");
            new ApiClient(AppState.settings).push(account.id, capture, code, friends);
            AppState.settings.target(account);
            runOnUiThread(() -> message("已推送到 " + account.name));
            if (code) watchOnline(account);
        });
    }
    private void createAccount(AppState.Capture capture, boolean friends) {
        if (capture == null || capture.code().isEmpty()) { message("新增账号需要 Code"); return; }
        String nonce = java.util.UUID.randomUUID().toString().replace("-", "").substring(0, 12);
        LinearLayout form = ui.column(); form.setPadding(ui.dp(24), ui.dp(8), ui.dp(24), 0);
        TextInputEditText name = ui.input(form, "账号备注", "", false);
        Ui.field(name).setHelperText("会自动附加随机后缀，避免与同名账号混淆");
        var dialog = new MaterialAlertDialogBuilder(this).setTitle("新增" + capture.platform().label + "账号")
                .setView(form).setNegativeButton("取消", null).setPositiveButton("新增", null).create();
        Ui.secure(dialog.getWindow());
        dialog.setOnShowListener(ignored -> dialog.getButton(-1).setOnClickListener(view -> {
            String baseName, actualName;
            try { baseName = AccountNamePolicy.base(Ui.value(name)); actualName = AccountNamePolicy.unique(baseName, nonce); Ui.field(name).setError(null); }
            catch (IllegalArgumentException error) { Ui.field(name).setError(error.getMessage()); return; }
            dialog.dismiss();
            String connection = AppState.settings.connectionId();
            async("正在新增账号", () -> {
                if (!connection.equals(AppState.settings.connectionId()) || !AppState.isCurrentCapture(capture))
                    throw new IllegalStateException("服务器或本次结果已变化，请重新推送");
                ApiClient.Account created = new ApiClient(AppState.settings).create(baseName, actualName, capture, friends);
                runOnUiThread(() -> message("已新增账号 " + created.name));
                watchOnline(created);
            });
        })); dialog.show();
    }
    /** The server accepting a Code is not the same as the account being online; report the latter separately. */
    private void watchOnline(ApiClient.Account account) {
        AppState.IO.execute(() -> {
            try {
                for (int attempt = 0; attempt < 4; attempt++) {
                    Thread.sleep(4000);
                    ApiClient client = new ApiClient(AppState.settings);
                    for (ApiClient.Account current : client.accounts()) if (current.id.equals(account.id) && client.connected(current)) {
                        onlineCache.put(account.id, true); AppState.log("服务器", "推送的账号已上线");
                        runOnUiThread(() -> { message(account.name + " 已上线"); if (page == 2) renderAccounts(); });
                        return;
                    }
                }
                AppState.log("服务器", "推送已完成，账号尚未上线");
                runOnUiThread(() -> message(account.name + " 尚未上线，可在账号页查看"));
            } catch (Exception ignored) { /* Online state stays visible on the accounts page. */ }
        });
    }
    private void manual(String initial) {
        CapturePlatform platform = AppState.settings.capturePlatform();
        LinearLayout form = ui.column(); form.setPadding(ui.dp(24), ui.dp(8), ui.dp(24), 0);
        TextInputEditText code = ui.input(form, "Code 或登录链接", initial, true);
        code.setId(R.id.manual_code);
        TextInputEditText gids = ui.input(form, "好友 GID（可选，逗号或换行分隔）", "", false); gids.setSingleLine(false); gids.setMinLines(2);
        gids.setId(R.id.manual_gids); gids.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS); gids.setGravity(Gravity.TOP);
        ScrollView formScroll = new ScrollView(this); formScroll.addView(form);
        var dialog = new MaterialAlertDialogBuilder(this).setTitle("手动导入 · " + platform.label).setView(formScroll)
                .setNegativeButton("取消", null).setPositiveButton("导入", null).create();
        Ui.secure(dialog.getWindow());
        dialog.setOnShowListener(d -> dialog.getButton(-1).setOnClickListener(v -> {
            try {
                if (platform != AppState.settings.capturePlatform()) throw new IllegalStateException("平台已变化，请重新导入");
                String value = Ui.value(code).isEmpty() ? "" : UrlPolicy.manualCode(Ui.value(code));
                List<String> friends = UrlPolicy.gids(Ui.value(gids), "");
                if (value.isEmpty() && friends.isEmpty()) throw new IllegalArgumentException("请至少填写 Code 或好友 GID");
                AppState.captured(value, "", friends, false, platform); AppState.status = "已手动导入";
                code.setText(""); gids.setText(""); dialog.dismiss();
                if (page != 1) navigation.setSelectedItemId(R.id.nav_capture); else refresh();
            } catch (Exception e) { message(e.getMessage()); }
        })); dialog.show();
    }

    // ---------------------------------------------------------------- 账号
    private void accountsPage() {
        LinearLayout bar = header("账号");
        bar.addView(ui.iconButton(R.drawable.ic_refresh, R.color.farm_ink, "刷新", this::loadAccounts));
        LinearLayout alerts = ui.card(content);
        monitorSwitch = new MaterialSwitch(this); monitorSwitch.setContentDescription("掉线提醒");
        monitorStatus = ui.listRow(alerts, ui.icon(R.drawable.ic_bell, R.color.farm_accent, 22), "掉线提醒", "", monitorSwitch, () -> monitorSwitch.toggle());
        monitorSwitch.setOnCheckedChangeListener((button, on) -> { if (button.getTag() == null) setMonitoring(on); });
        LinearLayout heading = ui.row(); heading.setPadding(ui.dp(6), ui.dp(16), 0, ui.dp(8));
        accountTitle = ui.text("服务器账号", 13, R.color.farm_muted); accountTitle.setTypeface(Ui.MEDIUM);
        heading.addView(accountTitle, new LinearLayout.LayoutParams(0, -2, 1));
        selectAll = ui.button(heading, "全选", Ui.Tone.TEXT, this::toggleAllWatched);
        ui.withIcon(selectAll, R.drawable.ic_select_all); selectAll.setMinHeight(ui.dp(36)); selectAll.setMinimumHeight(ui.dp(36));
        ui.add(content, heading, 0);
        accountList = ui.column(); ui.add(content, accountList, 0);
        if (!accountCacheConnection.equals(AppState.settings.connectionId())) { accountCache = new ArrayList<>(); onlineCache.clear(); }
        renderAccounts(); loadAccounts();
    }
    private void setMonitoring(boolean on) {
        if (on && !requireServer()) { refresh(); return; }
        AppState.settings.monitorEnabled(on);
        syncMonitor(on); refresh();
    }
    /**
     * Run the alert service exactly when the switch is on and at least one account is selected.
     * An enabled switch with nothing selected simply alerts nobody.
     */
    private void syncMonitor(boolean askPermission) {
        boolean want = AppState.settings.monitorEnabled() && !AppState.settings.monitoredIds().isEmpty()
                && !AppState.settings.server().isEmpty() && AppState.settings.hasToken();
        if (!want) { if (AppState.monitoring) stopService(new Intent(this, AccountMonitorService.class)); return; }
        if (AppState.monitoring) return;
        Runnable start = () -> startForegroundService(new Intent(this, AccountMonitorService.class).setAction("START"));
        if (notificationsAllowed()) start.run(); else if (askPermission) notificationPermission(start);
    }
    private boolean notificationsAllowed() {
        return (Build.VERSION.SDK_INT < 33 || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED)
                && NotificationManagerCompat.from(this).areNotificationsEnabled();
    }
    private void toggleAllWatched() {
        Set<String> watched = AppState.settings.monitoredIds();
        boolean all = !accountCache.isEmpty() && accountCache.stream().allMatch(a -> watched.contains(a.id));
        for (ApiClient.Account account : accountCache) { if (all) watched.remove(account.id); else watched.add(account.id); }
        AppState.settings.monitor(watched); syncMonitor(false);
        renderAccounts(); refresh();
    }
    private void loadAccounts() {
        if (AppState.settings.server().isEmpty() || !AppState.settings.hasToken()) { renderAccounts(); return; }
        int run = ++accountLoad; accountsLoading = true; renderAccounts();
        AppState.IO.execute(() -> {
            try {
                ApiClient client = new ApiClient(AppState.settings);
                List<ApiClient.Account> list = client.accounts();
                runOnUiThread(() -> {
                    if (run != accountLoad || !client.isCurrent()) return;
                    accountCache = list; accountCacheConnection = client.connectionId; accountsLoading = false;
                    onlineCache.keySet().retainAll(list.stream().map(a -> a.id).collect(java.util.stream.Collectors.toSet()));
                    renderAccounts();
                });
                for (ApiClient.Account account : list) {
                    if (run != accountLoad) return;
                    boolean online;
                    try { online = client.connected(account); } catch (Exception unknown) { continue; }
                    boolean state = online;
                    runOnUiThread(() -> { if (run != accountLoad) return; onlineCache.put(account.id, state); renderAccounts(); });
                }
            } catch (Exception error) {
                String detail = error instanceof ApiClient.AuthException ? "Token 无效，请到设置更新" : "无法连接服务器";
                runOnUiThread(() -> { if (run != accountLoad) return; accountsLoading = false; renderAccounts(); message(detail); });
            }
        });
    }
    private void renderAccounts() {
        if (accountList == null) return;
        accountList.removeAllViews();
        boolean configured = !AppState.settings.server().isEmpty() && AppState.settings.hasToken();
        selectAll.setVisibility(configured && !accountCache.isEmpty() ? View.VISIBLE : View.GONE);
        accountTitle.setText(accountCache.isEmpty() ? "服务器账号" : "服务器账号 · " + accountCache.size());
        if (!configured) {
            LinearLayout body = ui.padded(ui.card(accountList));
            ui.add(body, ui.text("连接服务器后，这里会显示账号和在线状态。", 14, R.color.farm_muted), 12);
            ui.button(body, "去设置", Ui.Tone.TONAL, this::configureServer); return;
        }
        if (accountCache.isEmpty()) {
            LinearLayout body = ui.padded(ui.card(accountList));
            ui.add(body, ui.text(accountsLoading ? "正在读取…" : "服务器上还没有账号。获取 Code 后推送即可新增。", 14, R.color.farm_muted), 4);
            return;
        }
        Set<String> watched = AppState.settings.monitoredIds();
        boolean all = accountCache.stream().allMatch(a -> watched.contains(a.id));
        selectAll.setText(all ? "全不选" : "全选");
        LinearLayout card = ui.card(accountList);
        for (ApiClient.Account account : accountCache) {
            Boolean online = onlineCache.get(account.id);
            boolean bell = watched.contains(account.id);
            ImageView toggle = ui.iconButton(bell ? R.drawable.ic_bell : R.drawable.ic_bell_off, bell ? R.color.farm_accent : R.color.farm_line_strong,
                    bell ? "关闭 " + account.name + " 的掉线提醒" : "开启 " + account.name + " 的掉线提醒", () -> toggleWatch(account));
            if (bell) toggle.setBackground(ui.oval(R.color.farm_tint));
            String state = online == null ? (account.running ? "检查中" : "已停止") : online ? "在线" : "离线";
            int tone = online == null ? R.color.farm_line_strong : online ? R.color.farm_accent : R.color.farm_danger;
            ui.listRow(card, ui.dot(tone), account.name, account.identityLabel() + " · " + state, toggle, () -> relogin(account));
        }
        TextView hint = ui.text("点账号重新登录，点铃铛开关该账号的掉线提醒。", 12, R.color.farm_muted);
        hint.setPadding(ui.dp(6), ui.dp(10), ui.dp(6), 0); ui.add(accountList, hint, 0);
    }
    private void toggleWatch(ApiClient.Account account) {
        Set<String> watched = AppState.settings.monitoredIds();
        if (!watched.remove(account.id)) watched.add(account.id);
        AppState.settings.monitor(watched); syncMonitor(false);
        renderAccounts(); refresh();
    }
    private void relogin(ApiClient.Account account) {
        CapturePlatform platform;
        try { platform = CapturePlatform.fromId(account.platform); } catch (IllegalArgumentException e) { message(e.getMessage()); return; }
        new MaterialAlertDialogBuilder(this).setTitle("重新登录「" + account.name + "」？")
                .setMessage(account.identityLabel() + "\n" + (platform == CapturePlatform.WECHAT ? "将打开微信扫码登录。" : "请先在 QQ 中切换到这个账号，然后开始获取。"))
                .setNegativeButton("取消", null).setPositiveButton("继续", (d, w) -> {
                    if (!switchPlatform(platform)) return;
                    AppState.settings.target(account);
                    if (platform == CapturePlatform.WECHAT) { wechatVpn = false; startActivity(new Intent(this, WxLoginActivity.class)); }
                    if (page == 1) showPage(); else navigation.setSelectedItemId(R.id.nav_capture);
                }).show();
    }

    // ---------------------------------------------------------------- 日志
    private void logsPage() {
        LinearLayout bar = header("日志");
        LinearLayout tools = ui.row();
        ui.button(tools, "导出", Ui.Tone.TEXT, R.drawable.ic_download, () -> export("logs"));
        ui.button(tools, "清空", Ui.Tone.DANGER, R.drawable.ic_delete, AppState::clearLogs);
        bar.addView(tools);
        LinearLayout body = ui.padded(ui.card(content));
        logs = ui.text("", 12, R.color.farm_ink); logs.setTypeface(Typeface.MONOSPACE); logs.setTextIsSelectable(true);
        logs.setLineSpacing(ui.dp(4), 1); body.addView(logs);
        View gap = new View(this); ui.add(content, gap, 12);
        LinearLayout details = ui.card(content);
        ImageView arrow = ui.icon(R.drawable.ic_expand, R.color.farm_muted, 22);
        TextView summary = ui.listRow(details, "连接详情", "TLS 主机与转发统计", arrow, null);
        View row = (View) summary.getParent().getParent();
        connections = ui.text("", 12, R.color.farm_muted); connections.setTypeface(Typeface.MONOSPACE); connections.setTextIsSelectable(true);
        connections.setPadding(ui.dp(16), 0, ui.dp(16), ui.dp(16)); connections.setVisibility(View.GONE); details.addView(connections);
        ui.ripple(row);
        row.setOnClickListener(v -> {
            boolean open = connections.getVisibility() != View.VISIBLE;
            connections.setVisibility(open ? View.VISIBLE : View.GONE); arrow.animate().rotation(open ? 180 : 0).setDuration(160).start();
        });
    }

    // ---------------------------------------------------------------- 设置
    private void settingsPage() {
        header("设置");
        ui.section(content, "服务器");
        LinearLayout server = ui.padded(ui.card(content));
        TextInputEditText address = ui.input(server, "服务器地址", AppState.settings.server(), false);
        address.setId(R.id.server_address);
        address.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        address.setImeOptions(EditorInfo.IME_ACTION_NEXT | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        Ui.field(address).setPlaceholderText("https://farm.example.com");
        TextInputEditText token = ui.input(server, AppState.settings.hasToken() ? "管理 Token（已保存，留空不变）" : "管理 Token", "", true);
        token.setId(R.id.server_token);
        MaterialSwitch http = new MaterialSwitch(this); http.setId(R.id.server_http); http.setChecked(AppState.settings.allowHttp());
        http.setText("允许 HTTP（明文）"); http.setTextSize(15); http.setTextColor(getColor(R.color.farm_ink)); http.setMinHeight(ui.dp(48));
        http.setOnCheckedChangeListener((button, on) -> {
            if (on && button.isPressed()) new MaterialAlertDialogBuilder(this).setTitle("允许 HTTP？")
                    .setMessage("HTTP 会明文传输管理 Token 和 Code，建议为服务器配置 HTTPS。")
                    .setNegativeButton("取消", (d, w) -> http.setChecked(false)).setPositiveButton("允许", null)
                    .setOnCancelListener(d -> http.setChecked(false)).show();
        });
        ui.add(server, http, 4);
        TextView feedback = ui.text(AppState.settings.hasToken() ? "Token 已保存" : "", 13, R.color.farm_muted);
        feedback.setId(R.id.server_feedback); feedback.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        feedback.setVisibility(AppState.settings.hasToken() ? View.VISIBLE : View.GONE); ui.add(server, feedback, 8);
        ui.button(server, "保存并测试", Ui.Tone.PRIMARY, () -> {
            String enteredToken = Ui.value(token), enteredAddress = Ui.value(address); boolean allowHttp = http.isChecked();
            Ui.field(address).setError(null); Ui.field(token).setError(null);
            try { UrlPolicy.server(enteredAddress, allowHttp); }
            catch (IllegalArgumentException error) { Ui.field(address).setError(error.getMessage()); address.requestFocus(); return; }
            if (enteredToken.isEmpty() && (!AppState.settings.hasToken()
                    || !UrlPolicy.server(enteredAddress, allowHttp).toString().equals(AppState.settings.server()))) {
                Ui.field(token).setError("请填写管理 Token"); token.requestFocus(); return;
            }
            dismissKeyboard();
            async("正在连接", () -> {
                boolean changed = AppState.settings.saveServer(enteredAddress, enteredToken, allowHttp);
                if (changed) { stopService(new Intent(this, AccountMonitorService.class)); AppState.clearCapture(); }
                runOnUiThread(() -> { token.setText(""); Ui.field(token).setHint("管理 Token（已保存，留空不变）"); });
                String result;
                try { result = new ApiClient(AppState.settings).ping(); }
                catch (Exception error) { result = error instanceof ApiClient.AuthException ? "已保存，但 Token 无效" : "已保存，但暂时无法连接"; }
                String text = result;
                runOnUiThread(() -> { feedback.setText(text); feedback.setVisibility(View.VISIBLE); });
            });
        }).setId(R.id.save_server);
        token.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        token.setOnEditorActionListener((v, action, event) -> {
            if (action != EditorInfo.IME_ACTION_DONE) return false;
            server.findViewById(R.id.save_server).performClick(); return true;
        });

        ui.section(content, "推送");
        LinearLayout push = ui.card(content);
        ui.switchRow(push, "获取后自动推送", "自动比对服务器账号，确认后推送", AppState.settings.autoPush(), AppState.settings::autoPush);
        ui.switchRow(push, "推送 Code", null, AppState.settings.pushCode(), AppState.settings::pushCode);
        ui.switchRow(push, "推送好友", null, AppState.settings.pushFriends(), AppState.settings::pushFriends);

        ui.section(content, "获取");
        LinearLayout capture = ui.card(content);
        overlaySwitch = ui.switchRow(capture, "悬浮窗", "在 QQ 中查看结果、推送和换号继续获取",
                AppState.settings.overlayEnabled() && Settings.canDrawOverlays(this), this::setOverlayEnabled);
        boolean wechat = AppState.settings.capturePlatform() == CapturePlatform.WECHAT;
        ui.switchRow(capture, wechat ? "微信 VPN 只读好友" : "同时读取好友", wechat ? "开启后不保留 Code" : null,
                AppState.settings.friendCapture(), AppState.settings::friendCapture);
        ui.switchRow(capture, "透传诊断", "不解密，只测试联网", AppState.settings.passthrough(), on -> { AppState.settings.passthrough(on); refresh(); })
                .setId(R.id.passthrough_mode);
        TextView certificate = ui.listRow(capture, "抓包证书", "读取中…", null, this::certificateSheet);
        AppState.IO.execute(() -> {
            String state = certificateState();
            runOnUiThread(() -> certificate.setText(state));
        });

        ui.section(content, "提醒");
        LinearLayout alerts = ui.card(content);
        TextView interval = ui.listRow(alerts, "检查间隔", AppState.settings.pollSeconds() + " 秒", null, null);
        ((View) interval.getParent().getParent()).setOnClickListener(v -> prompt("检查间隔（秒）", Integer.toString(AppState.settings.pollSeconds()), InputType.TYPE_CLASS_NUMBER, value -> {
            int seconds = Integer.parseInt(value);
            if (seconds < 1) throw new IllegalArgumentException("请输入正整数");
            AppState.settings.pollSeconds(seconds); interval.setText(seconds + " 秒");
        }));
        ui.ripple((View) interval.getParent().getParent());
        ui.listRow(alerts, "通知与后台", null, null, this::permissionSheet);

        ui.section(content, "隐私");
        LinearLayout privacy = ui.card(content);
        ui.switchRow(privacy, "允许截图", "关闭时截屏和录屏显示为黑色", AppState.settings.allowScreenshots(), on -> {
            AppState.settings.allowScreenshots(on); Ui.secure(getWindow());
        });

        ui.section(content, "更新");
        LinearLayout updates = ui.card(content);
        ui.listRow(updates, "检查更新", "当前 v" + BuildConfig.VERSION_NAME, null, this::checkUpdates);
        if (verifiedUpdate != null) ui.listRow(updates, "安装已下载的更新", null, null, this::installUpdate);
        ui.switchRow(updates, "下载加速", null, AppState.settings.accelerated(), AppState.settings::accelerated);
        TextView mirror = ui.listRow(updates, "加速地址", AppState.settings.mirror(), null, null);
        ((View) mirror.getParent().getParent()).setOnClickListener(v -> prompt("HTTPS 加速地址", AppState.settings.mirror(), InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI, value -> {
            String base = UrlPolicy.server(value, false).toString();
            AppState.settings.mirror(base); mirror.setText(base);
        }));
        ui.ripple((View) mirror.getParent().getParent());
        ui.listRow(updates, "源代码", null, null, () -> openWeb("https://github.com/" + UrlPolicy.REPOSITORY));

        LinearLayout danger = ui.row(); danger.setGravity(Gravity.CENTER); danger.setPadding(0, ui.dp(20), 0, 0);
        ui.button(danger, "清除服务器设置", Ui.Tone.DANGER, R.drawable.ic_delete, () -> new MaterialAlertDialogBuilder(this).setTitle("清除服务器设置？")
                .setMessage("将停止掉线提醒并删除服务器地址和 Token。服务器上的账号与已安装的证书不受影响。")
                .setNegativeButton("取消", null).setPositiveButton("清除", (d, w) -> {
                    stopService(new Intent(this, AccountMonitorService.class)); AppState.settings.monitorEnabled(false);
                    AppState.settings.forgetServer(); AppState.clearCapture(); showPage();
                }).show());
        ui.add(content, danger, 0);
    }
    private interface Accept { void apply(String value) throws Exception; }
    private void setOverlayEnabled(boolean enabled) {
        if (!enabled || Settings.canDrawOverlays(this)) {
            AppState.settings.overlayEnabled(enabled); AppState.changed(); return;
        }
        if (overlaySwitch != null) Ui.setQuietly(overlaySwitch, false);
        requestingOverlay = true;
        try { startActivity(new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:" + getPackageName()))); }
        catch (android.content.ActivityNotFoundException unavailable) {
            requestingOverlay = false; message("请在系统应用权限中允许农场助手显示悬浮窗");
        }
    }
    private void prompt(String title, String value, int inputType, Accept accept) {
        LinearLayout form = ui.column(); form.setPadding(ui.dp(24), ui.dp(8), ui.dp(24), 0);
        TextInputEditText edit = ui.input(form, title, value, false); edit.setInputType(inputType);
        var dialog = new MaterialAlertDialogBuilder(this).setTitle(title).setView(form)
                .setNegativeButton("取消", null).setPositiveButton("保存", null).create();
        dialog.setOnShowListener(d -> dialog.getButton(-1).setOnClickListener(v -> {
            try { accept.apply(Ui.value(edit)); dialog.dismiss(); }
            catch (Exception error) { Ui.field(edit).setError(error.getMessage() == null || error instanceof NumberFormatException ? "格式不正确" : error.getMessage()); }
        }));
        dialog.show();
    }
    private String certificateState() {
        CertificateManager manager = new CertificateManager(this);
        try {
            if (!manager.exists()) return "未生成";
            return manager.visibleInSystem() ? "已安装" : "已生成，未安装到系统";
        } catch (Exception e) { return "读取失败"; }
    }

    // ---------------------------------------------------------------- 刷新
    private void refresh() {
        AppState.Capture finished = AppState.capture();
        if (finished != null && AppState.hasPendingCapture() && !busy && !ApiClient.isPushing()
                && (!finished.ownGid().isEmpty() || finished.friendsObserved() || !AppState.captureCollecting)
                && finished.generation() != handledCapture) {
            handledCapture = finished.generation();
            if (AppState.settings.autoPush()) root.post(() -> syncCapture(true));
        }
        if (core != null) refreshCapture();
        if (overlaySwitch != null) Ui.setQuietly(overlaySwitch, AppState.settings.overlayEnabled() && Settings.canDrawOverlays(this));
        if (monitorSwitch != null) {
            boolean enabled = AppState.settings.monitorEnabled();
            monitorSwitch.setTag(Boolean.TRUE); monitorSwitch.setChecked(enabled); monitorSwitch.setTag(null);
            int count = AppState.settings.monitoredIds().size();
            String last = AppState.monitorLastCheckedAt == 0 ? "" : " · " + java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(new java.util.Date(AppState.monitorLastCheckedAt));
            monitorStatus.setText(!enabled ? (count == 0 ? "已关闭" : "已关闭 · 已选 " + count + " 个账号")
                    : count == 0 ? "已开启 · 未选择账号" : AppState.monitoring ? "正在提醒 " + count + " 个账号" + last : notificationsAllowed() ? "已开启 · 正在启动" : "已开启 · 需要允许通知");
            monitorStatus.setVisibility(View.VISIBLE);
        }
        if (connections != null) connections.setText(AppState.connectionText());
        if (logs != null) {
            boolean bottom = !scroll.canScrollVertically(1);
            String text = AppState.logText(); if (!text.contentEquals(logs.getText())) logs.setText(text.isEmpty() ? "暂无日志" : text);
            if (bottom) scroll.post(() -> scroll.fullScroll(View.FOCUS_DOWN));
        }
    }
    private void configureServer() { dismissKeyboard(); navigation.setSelectedItemId(R.id.nav_settings); }
    private boolean requireServer() {
        if (!AppState.settings.server().isEmpty() && AppState.settings.hasToken()) return true;
        configureServer(); message("请先填写服务器地址和 Token"); return false;
    }
    private void dismissKeyboard() {
        getSystemService(InputMethodManager.class).hideSoftInputFromWindow(root.getWindowToken(), 0); root.requestFocus();
    }

    // ---------------------------------------------------------------- 证书与权限
    private BottomSheetDialog sheet(String title, LinearLayout body) {
        BottomSheetDialog sheet = new BottomSheetDialog(this);
        ScrollView scroller = new ScrollView(this);
        LinearLayout frame = ui.column(); frame.setPadding(ui.dp(20), ui.dp(8), ui.dp(20), ui.dp(24));
        TextView heading = ui.strong(title, 20); heading.setPadding(ui.dp(4), 0, 0, ui.dp(12)); frame.addView(heading);
        frame.addView(body); scroller.addView(frame); sheet.setContentView(scroller);
        Ui.secure(sheet.getWindow());
        return sheet;
    }
    private void certificateSheet() {
        LinearLayout body = ui.column();
        BottomSheetDialog sheet = sheet("抓包证书", body);
        Runnable[] render = new Runnable[1];
        render[0] = () -> AppState.IO.execute(() -> {
            CertificateManager manager = new CertificateManager(this);
            boolean exists = manager.exists(); boolean installed = false; String fingerprint = "";
            try { if (exists) { installed = manager.visibleInSystem(); fingerprint = manager.fingerprint(); } } catch (Exception ignored) { }
            boolean visible = installed; String print = fingerprint;
            runOnUiThread(() -> {
                body.removeAllViews();
                LinearLayout card = ui.card(body);
                ui.listRow(card, "状态", !exists ? "未生成" : visible ? "已安装到系统" : "已生成，未安装到系统", new View(this), null);
                if (exists && !print.isEmpty()) {
                    TextView sub = ui.listRow(card, "SHA-256", print.length() > 23 ? print.substring(0, 23) + "…" : print, new View(this), null);
                    sub.setTypeface(Typeface.MONOSPACE); sub.setTextIsSelectable(true);
                }
                View gap = new View(this); ui.add(body, gap, 12);
                if (!exists) ui.button(body, "生成证书", Ui.Tone.PRIMARY, () -> async("正在生成证书", () -> {
                    new CertificateManager(this).generate(); AppState.log("证书", "已生成本地 CA；私钥保留在 Android Keystore，尚未安装到系统");
                    runOnUiThread(render[0]);
                }));
                else {
                    ui.button(body, "导出证书", visible ? Ui.Tone.TONAL : Ui.Tone.PRIMARY, () -> export("certificate"));
                    ui.button(body, "打开系统安装", Ui.Tone.TONAL, () -> openSettings(new Intent(Settings.ACTION_SECURITY_SETTINGS)));
                    if (!visible) ui.add(body, ui.text("在 安全 › 加密与凭据 › 安装证书 › CA 证书 中选择导出的文件。", 12, R.color.farm_muted), 8);
                    LinearLayout danger = ui.row(); danger.setGravity(Gravity.CENTER);
                    ui.button(danger, "删除本地证书", Ui.Tone.DANGER, () -> new MaterialAlertDialogBuilder(this).setTitle("删除本地证书？")
                            .setMessage("证书私钥将被永久删除，重新生成后需要重新安装。系统中已安装的副本需另行移除。")
                            .setNegativeButton("取消", null).setPositiveButton("删除", (d, w) -> async("正在删除", () -> {
                                if (AppState.capturing) throw new IllegalStateException("请先停止获取");
                                new CertificateManager(this).deleteLocal(); AppState.log("证书", "助手本地证书与私钥已删除，不可恢复；系统副本需手动移除");
                                runOnUiThread(render[0]);
                            })).show());
                    ui.add(body, danger, 0);
                }
            });
        });
        render[0].run();
        sheet.setOnDismissListener(d -> { if (page == 1 || page == 4) showPage(); });
        sheet.show();
    }
    private void permissionSheet() {
        LinearLayout body = ui.column();
        BottomSheetDialog sheet = sheet("通知与后台", body);
        LinearLayout card = ui.card(body);
        ui.listRow(card, "允许通知", null, null, () -> notificationPermission(() -> message("通知已允许")));
        ui.listRow(card, "系统通知设置", null, null, () -> openSettings(new Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, getPackageName())));
        ui.listRow(card, "电池优化", "被限制时提醒可能延迟", null, () -> openSettings(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)));
        sheet.show();
    }
    private void notificationPermission(Runnable action) {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            afterPermission = action; requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 11);
        } else if (!NotificationManagerCompat.from(this).areNotificationsEnabled()) {
            message("系统通知已关闭，请先开启");
            openSettings(new Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, getPackageName()));
        } else action.run();
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == 11) { Runnable action = afterPermission; afterPermission = null;
            if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED && action != null) action.run();
            else {
                if (AppState.settings.monitorEnabled() && !AppState.monitoring) AppState.settings.monitorEnabled(false);
                message("未允许通知，掉线提醒已关闭"); refresh();
            } }
    }
    private void export(String type) {
        if (type.equals("certificate") && !new CertificateManager(this).exists()) { message("请先生成证书"); return; }
        pendingExport = type;
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(type.equals("certificate") ? "application/pkix-cert" : "text/plain")
                .putExtra(Intent.EXTRA_TITLE, type.equals("certificate") ? "FarmHelper-LocalCA.cer" : "farm-helper-redacted-log.txt");
        startActivityForResult(intent, 12);
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == 10) { if (result == Activity.RESULT_OK) startCapture(); else message("未授权 VPN"); }
        if (request == 12 && result == Activity.RESULT_OK && data != null && data.getData() != null && pendingExport != null) {
            String type = pendingExport; pendingExport = null; Uri uri = data.getData();
            async("正在导出", () -> {
                byte[] bytes = type.equals("certificate") ? new CertificateManager(this).export() : AppState.logText().getBytes(java.nio.charset.StandardCharsets.UTF_8);
                try (var output = getContentResolver().openOutputStream(uri, "wt")) { if (output == null) throw new IllegalStateException("无法写入目标文件"); output.write(bytes); }
                runOnUiThread(() -> message(type.equals("certificate") ? "证书已导出，请到系统设置安装" : "日志已导出"));
            });
        }
    }
    private void checkUpdates() {
        async("正在检查更新", () -> {
            UpdateManager.Release release = UpdateManager.check(AppState.settings);
            runOnUiThread(() -> {
                if (release == null || UpdateManager.compare(release.version(), BuildConfig.VERSION_NAME) <= 0) { message("已是最新版本"); return; }
                new MaterialAlertDialogBuilder(this).setTitle("新版本 v" + release.version()).setMessage(release.notes().isEmpty() ? "下载后会校验签名，再由系统安装。" : release.notes())
                        .setNegativeButton("稍后", null).setPositiveButton("下载", (d, w) -> async("正在下载并校验", () -> {
                            File file = UpdateManager.download(this, AppState.settings, release); verifiedUpdate = file;
                            runOnUiThread(() -> new MaterialAlertDialogBuilder(this).setTitle("更新已就绪").setMessage("签名校验通过，现在安装？")
                                    .setNegativeButton("稍后", (dialog, which) -> showPage()).setPositiveButton("安装", (dialog, which) -> installUpdate()).show());
                        })).show();
            });
        });
    }
    private void installUpdate() {
        if (verifiedUpdate == null || !verifiedUpdate.isFile()) { message("请先下载更新"); return; }
        if (!getPackageManager().canRequestPackageInstalls()) {
            new MaterialAlertDialogBuilder(this).setTitle("需要安装权限").setMessage("授权后返回设置页，点「安装已下载的更新」。")
                    .setNegativeButton("取消", null).setPositiveButton("去授权", (d, w) -> {
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
    @androidx.annotation.IdRes private static int navId(int page) {
        switch (page) { case 2: return R.id.nav_accounts; case 3: return R.id.nav_logs; case 4: return R.id.nav_settings; default: return R.id.nav_capture; }
    }
    private void message(String text) { if (!isFinishing() && !isDestroyed()) Toast.makeText(this, text, Toast.LENGTH_LONG).show(); }
    private void openSettings(Intent intent) { try { startActivity(intent); } catch (Exception e) { message("系统未提供此入口"); } }
    private void openWeb(String address) {
        try { Uri uri = Uri.parse(UrlPolicy.server(address, AppState.settings.allowHttp()).toString()); startActivity(new Intent(Intent.ACTION_VIEW, uri)); }
        catch (Exception e) { message("地址无效或没有可用浏览器"); }
    }
}
