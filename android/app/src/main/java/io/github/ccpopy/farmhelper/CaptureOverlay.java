package io.github.ccpopy.farmhelper;

import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.graphics.PixelFormat;
import android.hardware.display.DisplayManager;
import android.net.VpnService;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.view.ContextThemeWrapper;
import android.view.Display;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.WindowManager;
import android.view.inputmethod.InputMethodManager;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.textfield.TextInputEditText;
import java.util.ArrayList;
import java.util.List;

/**
 * A user-enabled companion window. Collapsed it is a small draggable bubble; expanded it is a
 * compact translucent card that can start or stop the capture and sync what has been collected.
 */
public final class CaptureOverlay {
    private static final float BUBBLE_ALPHA = .82f, PANEL_ALPHA = .94f;
    private final Context context;
    private final WindowManager windows;
    private final Ui ui;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final Runnable listener = this::refresh;
    private FrameLayout root, bubbleFrame;
    private LinearLayout bubble, panel, body;
    private ImageView bubbleIcon;
    private TextView bubbleBadge, codeValue, friendValue, detail;
    private View statusDot;
    private MaterialButton power, sync, friends, clear;
    private WindowManager.LayoutParams layout;
    private boolean closed, attached, collapsed = true, busy, editing, formInput;
    private int x = -1, y = -1;
    private String message = "";
    private long messageGeneration = -1;
    private AppState.Capture selected;
    private ApiClient client;
    private List<ApiClient.Account> accounts = List.of();
    private boolean sendCode, sendFriends;
    private final Runnable tick = new Runnable() {
        @Override public void run() { if (!closed) { refresh(); main.postDelayed(this, 1_000); } }
    };

    public CaptureOverlay(Service service) {
        Context windowContext = service;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            try {
                Display display = service.getSystemService(DisplayManager.class).getDisplay(Display.DEFAULT_DISPLAY);
                if (display != null) windowContext = service.createDisplayContext(display)
                        .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null);
            } catch (RuntimeException unavailableDisplay) { /* Overlay support must never prevent capture. */ }
        }
        context = new ContextThemeWrapper(windowContext, R.style.Theme_FarmHelper);
        windows = context.getSystemService(WindowManager.class);
        ui = new Ui(context);
        AppState.listen(listener);
        main.post(tick);
    }

    // ------------------------------------------------------------------ 状态
    public void refresh() {
        if (Looper.myLooper() != Looper.getMainLooper()) { main.post(this::refresh); return; }
        if (closed) return;
        if (!AppState.settings.overlayEnabled() || !Settings.canDrawOverlays(context) || AppState.appVisible) { removeWindow(); return; }
        if (root == null) createWindow();
        if (root == null) return;
        constrain();
        int desired = windowFlags(editing);
        if (layout.flags != desired) { layout.flags = desired; updateWindow(); }
        float alpha = collapsed ? BUBBLE_ALPHA : PANEL_ALPHA;
        if (layout.alpha != alpha) { layout.alpha = alpha; updateWindow(); }

        if (selected != null && !busy && !selectionCurrent()) {
            selected = null; message = "结果已更新，请重新选择"; showResults();
        }
        AppState.Capture capture = AppState.capture();
        if (capture != null && messageGeneration != capture.generation() && selected == null && !busy) {
            message = ""; messageGeneration = capture.generation();
        }
        int count = capture == null ? 0 : capture.gids().size();
        boolean hasCode = capture != null && !capture.code().isEmpty();

        bubbleIcon.setImageResource(hasCode || count > 0 ? R.drawable.ic_check
                : AppState.capturing ? R.drawable.ic_power : R.drawable.ic_farm_leaf);
        bubble.setBackground(ui.oval(AppState.capturing || hasCode || count > 0 ? R.color.farm_accent : R.color.farm_card));
        bubbleIcon.setImageTintList(android.content.res.ColorStateList.valueOf(
                ui.color(AppState.capturing || hasCode || count > 0 ? R.color.farm_on_accent : R.color.farm_accent)));
        bubbleBadge.setText(count > 0 ? String.valueOf(count) : "");
        bubbleBadge.setVisibility(count > 0 ? View.VISIBLE : View.GONE);

        if (detail != null) {
            statusDot.setBackground(ui.oval(AppState.capturing ? R.color.farm_accent : R.color.farm_line_strong));
            codeValue.setText(hasCode ? "已取得" : AppState.capturing ? "等待中" : "—");
            friendValue.setText(String.valueOf(count));
            detail.setText(busy ? "正在同步…" : !message.isEmpty() ? message : AppState.status);
            power.setText(AppState.capturing ? "停止" : "开始");
            power.setEnabled(!busy);
            boolean available = !busy && !ApiClient.isPushing();
            sync.setEnabled(available && ((hasCode && AppState.settings.pushCode()) || (count > 0 && AppState.settings.pushFriends())));
            friends.setEnabled(available && count > 0);
            clear.setEnabled(available && capture != null);
        }
    }

    /** Keep the window on screen after rotation, resize, or a collapse/expand. */
    private void constrain() {
        int screenWidth = context.getResources().getDisplayMetrics().widthPixels;
        int screenHeight = context.getResources().getDisplayMetrics().heightPixels;
        int width = width(), height = Math.max(ui.dp(56), root.getHeight());
        if (x < 0) { x = screenWidth - width - ui.dp(12); y = screenHeight / 3; }
        int boundedX = Math.max(0, Math.min(x, screenWidth - width));
        int boundedY = Math.max(0, Math.min(y, screenHeight - height));
        if (layout.width != width || layout.x != boundedX || layout.y != boundedY) {
            layout.width = width; layout.x = x = boundedX; layout.y = y = boundedY; updateWindow();
        }
    }
    private int width() {
        if (collapsed) return ui.dp(56);
        int available = context.getResources().getDisplayMetrics().widthPixels - ui.dp(24);
        return Math.min(ui.dp(284), Math.max(ui.dp(200), available));
    }
    private int windowFlags(boolean focusable) {
        int flags = WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL;
        if (!focusable) flags |= WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE;
        if (!AppState.settings.allowScreenshots()) flags |= WindowManager.LayoutParams.FLAG_SECURE;
        return flags;
    }

    // ------------------------------------------------------------------ 窗口
    private void createWindow() {
        root = new FrameLayout(context);

        bubble = ui.column(); bubble.setGravity(Gravity.CENTER); bubble.setElevation(ui.dp(6));
        bubbleIcon = new ImageView(context);
        bubble.addView(bubbleIcon, new LinearLayout.LayoutParams(ui.dp(26), ui.dp(26)));
        bubble.setContentDescription("农场助手，点击展开，拖动可移动");
        installDrag(bubble, this::expand);
        bubbleFrame = new FrameLayout(context);
        bubbleFrame.addView(bubble, new FrameLayout.LayoutParams(ui.dp(56), ui.dp(56)));
        bubbleBadge = ui.text("", 11, R.color.farm_on_accent); bubbleBadge.setTypeface(Ui.MEDIUM);
        bubbleBadge.setGravity(Gravity.CENTER); bubbleBadge.setBackground(ui.oval(R.color.farm_amber));
        bubbleBadge.setMinWidth(ui.dp(18)); bubbleBadge.setPadding(ui.dp(4), ui.dp(1), ui.dp(4), ui.dp(1));
        bubbleBadge.setElevation(ui.dp(7));
        bubbleFrame.addView(bubbleBadge, new FrameLayout.LayoutParams(-2, ui.dp(18), Gravity.END | Gravity.TOP));
        root.addView(bubbleFrame, new FrameLayout.LayoutParams(-2, -2));

        panel = ui.column(); panel.setPadding(ui.dp(14), ui.dp(10), ui.dp(14), ui.dp(12));
        panel.setBackground(ui.shape(R.color.farm_card, 22)); panel.setElevation(ui.dp(8));
        panel.setVisibility(View.GONE);
        root.addView(panel, new FrameLayout.LayoutParams(-1, -2));

        LinearLayout header = ui.row();
        statusDot = ui.dot(R.color.farm_line_strong);
        LinearLayout.LayoutParams dotParams = new LinearLayout.LayoutParams(ui.dp(8), ui.dp(8));
        dotParams.setMarginEnd(ui.dp(8)); header.addView(statusDot, dotParams);
        TextView handle = ui.strong("农场助手", 14);
        handle.setMinHeight(ui.dp(36)); handle.setGravity(Gravity.CENTER_VERTICAL);
        handle.setContentDescription("拖动可移动悬浮窗");
        installDrag(handle, null);
        header.addView(handle, new LinearLayout.LayoutParams(0, -2, 1));
        header.addView(ui.iconButton(R.drawable.ic_expand, R.color.farm_muted, 36, "收起为悬浮球", this::collapse));
        header.addView(ui.iconButton(R.drawable.ic_close, R.color.farm_muted, 36, "关闭悬浮窗", () -> {
            AppState.settings.overlayEnabled(false); removeWindow(); AppState.changed();
        }));
        panel.addView(header);
        body = ui.column(); panel.addView(body);

        layout = new WindowManager.LayoutParams(width(), -2, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                windowFlags(false), PixelFormat.TRANSLUCENT);
        layout.gravity = Gravity.TOP | Gravity.START;
        layout.alpha = collapsed ? BUBBLE_ALPHA : PANEL_ALPHA;
        layout.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE;
        layout.setTitle("农场助手");
        constrain();
        try { windows.addView(root, layout); attached = true; showResults(); applyCollapsed(); }
        catch (RuntimeException denied) { removeWindow(); }
    }

    private void applyCollapsed() {
        if (root == null) return;
        bubbleFrame.setVisibility(collapsed ? View.VISIBLE : View.GONE);
        panel.setVisibility(collapsed ? View.GONE : View.VISIBLE);
        setEditing(formInput && !collapsed);
        layout.width = width(); updateWindow(); refresh();
    }
    private void expand() { if (collapsed) { collapsed = false; applyCollapsed(); } }
    private void collapse() { if (!collapsed) { collapsed = true; showResults(); applyCollapsed(); } }

    /** Drag moves the window; a tap without movement runs the click action. */
    private void installDrag(View handle, Runnable click) {
        handle.setOnTouchListener(new View.OnTouchListener() {
            float downX, downY; int originX, originY; boolean moved;
            @Override public boolean onTouch(View view, MotionEvent event) {
                if (layout == null) return false;
                switch (event.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        downX = event.getRawX(); downY = event.getRawY(); originX = layout.x; originY = layout.y;
                        moved = false; return true;
                    case MotionEvent.ACTION_MOVE:
                        float dx = event.getRawX() - downX, dy = event.getRawY() - downY;
                        if (Math.abs(dx) + Math.abs(dy) > ViewConfiguration.get(context).getScaledTouchSlop()) moved = true;
                        if (moved) {
                            int screenWidth = context.getResources().getDisplayMetrics().widthPixels;
                            int screenHeight = context.getResources().getDisplayMetrics().heightPixels;
                            layout.x = Math.max(0, Math.min(screenWidth - layout.width, originX + Math.round(dx)));
                            layout.y = Math.max(0, Math.min(screenHeight - Math.max(ui.dp(56), root.getHeight()), originY + Math.round(dy)));
                            x = layout.x; y = layout.y; updateWindow();
                        }
                        return true;
                    case MotionEvent.ACTION_UP:
                        if (!moved && click != null) { view.performClick(); click.run(); }
                        return true;
                    case MotionEvent.ACTION_CANCEL: return true;
                    default: return false;
                }
            }
        });
    }

    // ------------------------------------------------------------------ 内容
    private void resetBody() {
        formInput = false; setEditing(false); body.removeAllViews(); codeValue = friendValue = null; detail = null;
    }

    private void showResults() {
        if (body == null) return;
        resetBody(); selected = null;
        LinearLayout stats = ui.row();
        codeValue = (TextView) ui.statTile(stats, "—", "Code").getChildAt(0);
        friendValue = (TextView) ui.statTile(stats, "0", "好友").getChildAt(0);
        ui.add(body, stats, 8);
        detail = ui.text("", 12, R.color.farm_muted); detail.setMaxLines(2); ui.add(body, detail, 10);
        LinearLayout actions = ui.row();
        power = ui.button(actions, "开始", Ui.Tone.TONAL, this::toggleCapture);
        sync = ui.button(actions, "同步", Ui.Tone.PRIMARY, () -> choose(false));
        ui.equalize(actions);
        for (int i = 0; i < actions.getChildCount(); i++) ((MaterialButton) actions.getChildAt(i)).setMinHeight(ui.dp(46));
        ui.add(body, actions, 8);
        LinearLayout minor = ui.row();
        friends = ui.button(minor, "只同步好友", Ui.Tone.TEXT, () -> choose(true));
        clear = ui.button(minor, "清除", Ui.Tone.DANGER, () -> {
            if (busy || ApiClient.isPushing()) return;
            AppState.clearCapture(); message = "已清除，可换号继续获取"; messageGeneration = -1; refresh();
        });
        ui.equalize(minor);
        for (int i = 0; i < minor.getChildCount(); i++) ((MaterialButton) minor.getChildAt(i)).setMinHeight(ui.dp(38));
        ui.add(body, minor, 0);
        refresh();
    }

    /** Start or stop the capture without leaving the game. */
    private void toggleCapture() {
        if (busy) return;
        if (AppState.capturing) {
            context.startService(new Intent(context, CaptureVpnService.class).setAction("STOP"));
            message = "正在停止…"; refresh(); return;
        }
        if (!AppState.settings.passthrough() && !new CertificateManager(context).exists()) {
            message = "请先在应用内生成并安装证书"; refresh(); return;
        }
        // Consent is per-app and already granted for a repeat start; only a revoked grant needs the activity.
        if (VpnService.prepare(context) != null) {
            message = "请在助手中确认 VPN 授权";
            context.startActivity(new Intent(context, MainActivity.class)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP)
                    .putExtra("startVpn", true));
            refresh(); return;
        }
        context.startForegroundService(new Intent(context, CaptureVpnService.class).setAction("START"));
        message = ""; refresh();
    }

    private boolean selectionCurrent() {
        return selected != null && client != null && client.isCurrent() && AppState.isCurrentCapture(selected);
    }

    private void choose(boolean friendsOnly) {
        if (busy || ApiClient.isPushing()) return;
        AppState.Capture capture = AppState.capture();
        if (capture == null) { message = "还没有结果"; refresh(); return; }
        sendCode = !friendsOnly && AppState.settings.pushCode() && !capture.code().isEmpty();
        sendFriends = !capture.gids().isEmpty() && (friendsOnly || AppState.settings.pushFriends());
        if (!sendCode && !sendFriends) { message = "没有可同步的内容"; refresh(); return; }
        selected = capture; busy = true;
        try { client = new ApiClient(AppState.settings); }
        catch (Exception error) { fail(error); return; }
        refresh();
        AppState.IO.execute(() -> {
            try {
                List<ApiClient.Account> loaded = new ArrayList<>(client.accounts());
                loaded.removeIf(account -> !account.platform.equals(capture.platform().id));
                main.post(() -> {
                    busy = false;
                    if (closed || root == null) return;
                    if (!selectionCurrent()) { stale(); return; }
                    accounts = loaded;
                    ApiClient.Account detected = client.matchingAccount(accounts, capture);
                    if (detected != null) showConfirmation(detected, true); else showAccounts();
                });
            } catch (Exception error) { main.post(() -> fail(error)); }
        });
    }

    private String summary() {
        return (sendCode ? "更新 Code" : "") + (sendCode && sendFriends ? " · " : "")
                + (sendFriends ? "好友 " + selected.gids().size() + " 位" : "");
    }

    private void showAccounts() {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); ui.add(body, ui.strong("选择账号", 15), 2);
        ui.add(body, ui.text(summary(), 12, R.color.farm_muted), 8);
        ScrollView scroll = new ScrollView(context); LinearLayout list = ui.column(); scroll.addView(list);
        if (accounts.isEmpty()) ui.add(list, ui.text("服务器暂无同平台账号", 13, R.color.farm_muted), 8);
        for (ApiClient.Account account : accounts)
            ui.button(list, account.selectionLabel(), Ui.Tone.TONAL, () -> showConfirmation(account, false));
        int height = accounts.isEmpty() ? ui.dp(40) : Math.min(ui.dp(200), ui.dp(accounts.size() * 60));
        body.addView(scroll, new LinearLayout.LayoutParams(-1, height));
        LinearLayout actions = ui.row(); ui.add(body, actions, 0);
        if (sendCode) ui.button(actions, "新增账号", Ui.Tone.TEXT, this::showCreate);
        ui.button(actions, "返回", Ui.Tone.TEXT, this::showResults);
    }

    private void showConfirmation(ApiClient.Account account, boolean detected) {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); ui.add(body, ui.strong(detected ? "已匹配账号" : "确认同步", 15), 4);
        ui.add(body, ui.text(account.selectionLabel(), 14, R.color.farm_ink), 2);
        ui.add(body, ui.text(summary() + (sendCode ? " · 将替换该账号的登录" : " · 保留已有好友"), 12, R.color.farm_muted), 10);
        ui.button(body, "确认同步", Ui.Tone.PRIMARY, () -> submit(account));
        LinearLayout actions = ui.row(); ui.add(body, actions, 0);
        ui.button(actions, "换账号", Ui.Tone.TEXT, this::showAccounts);
        ui.button(actions, "取消", Ui.Tone.TEXT, this::showResults);
    }

    private void submit(ApiClient.Account account) {
        if (busy || ApiClient.isPushing()) return;
        if (!selectionCurrent()) { stale(); return; }
        AppState.Capture snapshot = selected;
        ApiClient api = client;
        boolean code = sendCode, gids = sendFriends;
        busy = true; showProgress();
        AppState.IO.execute(() -> {
            try {
                api.push(account.id, snapshot, code, gids);
                if (api.isCurrent()) AppState.settings.target(account);
                main.post(() -> finish("同步完成"));
            } catch (Exception error) { main.post(() -> fail(error)); }
        });
    }

    private void showCreate() {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); ui.add(body, ui.strong("新增账号", 15), 4);
        ui.add(body, ui.text(summary() + " · 备注会附加手机标识", 12, R.color.farm_muted), 6);
        TextInputEditText name = ui.input(body, "账号备注", "", false);
        ui.button(body, "新增并同步", Ui.Tone.PRIMARY, () -> {
            if (busy || ApiClient.isPushing()) return;
            if (!selectionCurrent()) { stale(); return; }
            String base, actual;
            try {
                base = AccountNamePolicy.base(Ui.value(name));
                actual = AccountNamePolicy.unique(base, java.util.UUID.randomUUID().toString().replace("-", "").substring(0, 12));
            } catch (IllegalArgumentException invalid) { Ui.field(name).setError(invalid.getMessage()); return; }
            AppState.Capture snapshot = selected; ApiClient api = client; boolean gids = sendFriends;
            busy = true; showProgress();
            AppState.IO.execute(() -> {
                try { api.create(base, actual, snapshot, gids); main.post(() -> finish("新账号已同步")); }
                catch (Exception error) { main.post(() -> fail(error)); }
            });
        });
        ui.button(body, "返回", Ui.Tone.TEXT, this::showAccounts);
        formInput = true; setEditing(!collapsed);
    }

    private void showProgress() {
        resetBody(); ui.add(body, ui.strong("正在同步…", 15), 4);
        ui.add(body, ui.text("完成后可继续获取，或清除结果换号。", 12, R.color.farm_muted), 6);
    }

    private void finish(String text) {
        busy = false; message = text;
        if (selected != null) messageGeneration = selected.generation();
        if (!closed && root != null) showResults();
        AppState.changed();
    }

    private void fail(Exception error) {
        busy = false;
        message = error instanceof java.io.IOException || error instanceof IllegalArgumentException
                ? error.getMessage() : "操作未完成，请检查服务器设置";
        if (message == null || message.isBlank()) message = "同步未完成，请重试";
        if (selected != null) messageGeneration = selected.generation();
        if (!closed && root != null) showResults();
    }

    private void stale() { selected = null; message = "结果或服务器配置已变更，请重新选择"; showResults(); }

    private void setEditing(boolean value) {
        if (!value && editing && root != null)
            context.getSystemService(InputMethodManager.class).hideSoftInputFromWindow(root.getWindowToken(), 0);
        editing = value;
        if (layout != null) { layout.flags = windowFlags(value); updateWindow(); }
    }

    private void updateWindow() {
        if (root == null || !attached) return;
        FrameLayout current = root;
        try { windows.updateViewLayout(root, layout); }
        catch (RuntimeException revoked) {
            // Finish this render before clearing its view references.
            main.post(() -> { if (root == current) removeWindow(); });
        }
    }

    private void removeWindow() {
        if (root == null) return;
        if (editing) context.getSystemService(InputMethodManager.class).hideSoftInputFromWindow(root.getWindowToken(), 0);
        boolean wasAttached = attached; attached = false;
        FrameLayout removed = root; root = null; bubbleFrame = null; panel = null; body = null; codeValue = friendValue = null; detail = null; editing = false;
        if (!wasAttached) return;
        try { windows.removeViewImmediate(removed); } catch (RuntimeException alreadyRemoved) { /* Permission may have been revoked. */ }
    }

    public void close() {
        closed = true; AppState.unlisten(listener); main.removeCallbacks(tick); removeWindow();
    }
}
