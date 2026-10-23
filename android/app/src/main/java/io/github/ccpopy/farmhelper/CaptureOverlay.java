package io.github.ccpopy.farmhelper;

import android.app.Service;
import android.content.Context;
import android.graphics.PixelFormat;
import android.hardware.display.DisplayManager;
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
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.textfield.TextInputEditText;
import java.util.ArrayList;
import java.util.List;

/** A user-enabled companion window owned by the existing VPN service. */
public final class CaptureOverlay {
    private final Context context;
    private final WindowManager windows;
    private final Ui ui;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final Runnable listener = this::refresh;
    private LinearLayout panel, body;
    private TextView title, result, detail;
    private MaterialButton sync, friends, clear;
    private WindowManager.LayoutParams layout;
    private boolean closed, collapsed, busy, editing, formInput;
    private int x = 16, y = 120;
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
            } catch (RuntimeException unavailableDisplay) { /* Overlay support must never prevent VPN startup. */ }
        }
        context = new ContextThemeWrapper(windowContext, R.style.Theme_FarmHelper);
        windows = context.getSystemService(WindowManager.class);
        ui = new Ui(context);
        AppState.listen(listener);
        main.post(tick);
    }

    public void refresh() {
        if (Looper.myLooper() != Looper.getMainLooper()) { main.post(this::refresh); return; }
        if (closed) return;
        if (!AppState.capturing || !AppState.settings.overlayEnabled() || !Settings.canDrawOverlays(context)) {
            removeWindow(); return;
        }
        if (panel == null) createWindow();
        if (panel == null) return;
        int screenHeight = context.getResources().getDisplayMetrics().heightPixels;
        int screenWidth = context.getResources().getDisplayMetrics().widthPixels;
        int constrainedX = Math.max(0, Math.min(layout.x, screenWidth - width()));
        int constrainedY = Math.max(0, Math.min(layout.y, screenHeight - Math.max(ui.dp(80), panel.getHeight())));
        if (layout.width != width() || layout.x != constrainedX || layout.y != constrainedY) {
            layout.width = width(); layout.x = x = constrainedX; layout.y = y = constrainedY; updateWindow();
        }
        int desired = windowFlags(editing);
        if (layout.flags != desired) { layout.flags = desired; updateWindow(); }
        if (selected != null && !busy && !selectionCurrent()) {
            selected = null; message = "结果已清除或更新，请重新选择"; showResults();
        }
        AppState.Capture capture = AppState.capture();
        if (capture != null && messageGeneration != capture.generation() && selected == null && !busy) {
            message = ""; messageGeneration = capture.generation();
        }
        String codeState = capture == null ? "等待 Code" : !capture.code().isEmpty() ? "Code 已获取" : "Code 暂无待推送";
        int count = capture == null ? 0 : capture.gids().size();
        title.setText(collapsed ? "农场 · " + (count > 0 ? count + " 位好友" : codeState) : "农场助手");
        if (result != null) {
            result.setText(codeState + "\n待同步好友 " + count + " 位");
            detail.setText(busy ? "正在同步，请稍候…" : !message.isEmpty() ? message
                    : capture != null && (!capture.code().isEmpty() || count > 0)
                    ? "可随时同步已有结果，VPN 会继续获取" : AppState.status);
            boolean available = !busy && !ApiClient.isPushing();
            sync.setEnabled(available && capture != null && ((!capture.code().isEmpty() && AppState.settings.pushCode())
                    || (count > 0 && AppState.settings.pushFriends())));
            friends.setEnabled(available && count > 0);
            clear.setEnabled(available && capture != null);
        }
    }

    private int windowFlags(boolean focusable) {
        int flags = WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL;
        if (!focusable) flags |= WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE;
        if (!AppState.settings.allowScreenshots()) flags |= WindowManager.LayoutParams.FLAG_SECURE;
        return flags;
    }

    private void createWindow() {
        panel = ui.column(); panel.setPadding(ui.dp(12), ui.dp(4), ui.dp(12), ui.dp(12));
        panel.setBackground(ui.shape(R.color.farm_card, 20)); panel.setElevation(ui.dp(10));
        LinearLayout header = ui.row();
        title = ui.strong("农场助手", 15); title.setMinHeight(ui.dp(48)); title.setGravity(Gravity.CENTER_VERTICAL);
        title.setContentDescription("拖动悬浮窗，点击收起或展开");
        title.setOnClickListener(v -> toggleCollapsed());
        installDrag(title); header.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
        MaterialButton hide = ui.button("关闭", Ui.Tone.TEXT, () -> {
            AppState.settings.overlayEnabled(false); removeWindow(); AppState.changed();
        });
        hide.setContentDescription("关闭悬浮窗，VPN 继续运行"); header.addView(hide);
        panel.addView(header);
        body = ui.column(); panel.addView(body);
        layout = new WindowManager.LayoutParams(width(), -2, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                windowFlags(false), PixelFormat.TRANSLUCENT);
        layout.gravity = Gravity.TOP | Gravity.START; layout.x = x; layout.y = y;
        layout.softInputMode = WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE;
        layout.setTitle("农场助手结果");
        try { windows.addView(panel, layout); showResults(); body.setVisibility(collapsed ? View.GONE : View.VISIBLE); }
        catch (RuntimeException denied) { removeWindow(); }
    }

    private int width() {
        int available = context.getResources().getDisplayMetrics().widthPixels - ui.dp(24);
        return Math.min(ui.dp(collapsed ? 240 : 340), Math.max(ui.dp(180), available));
    }

    private void toggleCollapsed() {
        collapsed = !collapsed; body.setVisibility(collapsed ? View.GONE : View.VISIBLE);
        setEditing(formInput && !collapsed);
        layout.width = width(); updateWindow(); refresh();
    }

    private void installDrag(View handle) {
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
                            layout.y = Math.max(0, Math.min(screenHeight - Math.max(ui.dp(80), panel.getHeight()), originY + Math.round(dy)));
                            x = layout.x; y = layout.y; updateWindow();
                        }
                        return true;
                    case MotionEvent.ACTION_UP:
                        if (!moved) view.performClick(); return true;
                    case MotionEvent.ACTION_CANCEL: return true;
                    default: return false;
                }
            }
        });
    }

    private void resetBody() {
        formInput = false; setEditing(false); body.removeAllViews(); result = null; detail = null;
    }

    private void showResults() {
        if (body == null) return;
        resetBody(); selected = null;
        result = ui.strong("等待获取结果", 15); ui.add(body, result, 6);
        detail = ui.text("", 12, R.color.farm_muted); ui.add(body, detail, 12);
        sync = ui.button(body, "同步已有结果", Ui.Tone.PRIMARY, () -> choose(false));
        LinearLayout actions = ui.row(); body.addView(actions);
        friends = ui.button(actions, "只同步好友", Ui.Tone.TEXT, () -> choose(true));
        clear = ui.button(actions, "清除结果", Ui.Tone.TEXT, () -> {
            if (busy || ApiClient.isPushing()) return;
            AppState.clearCapture(); message = "已清除，可切换账号继续获取"; messageGeneration = -1; refresh();
        });
        refresh();
    }

    private boolean selectionCurrent() {
        return selected != null && client != null && client.isCurrent() && AppState.isCurrentCapture(selected);
    }

    private void choose(boolean friendsOnly) {
        if (busy || ApiClient.isPushing()) return;
        AppState.Capture capture = AppState.capture();
        if (capture == null) { message = "请先获取结果"; refresh(); return; }
        sendCode = !friendsOnly && AppState.settings.pushCode() && !capture.code().isEmpty();
        sendFriends = !capture.gids().isEmpty() && (friendsOnly || AppState.settings.pushFriends());
        if (!sendCode && !sendFriends) { message = "没有选择可同步的内容"; refresh(); return; }
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
                    if (closed || panel == null) return;
                    if (!selectionCurrent()) { stale(); return; }
                    accounts = loaded;
                    ApiClient.Account detected = client.matchingAccount(accounts, capture);
                    if (detected != null) showConfirmation(detected, true); else showAccounts();
                });
            } catch (Exception error) { main.post(() -> fail(error)); }
        });
    }

    private String summary() {
        return (sendCode ? "更新 Code" : "") + (sendCode && sendFriends ? "，" : "")
                + (sendFriends ? "同步 " + selected.gids().size() + " 位好友" : "");
    }

    private void showAccounts() {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); ui.add(body, ui.strong("选择同步账号", 16), 8);
        ui.add(body, ui.text(summary(), 12, R.color.farm_muted), 8);
        ScrollView scroll = new ScrollView(context); LinearLayout list = ui.column(); scroll.addView(list);
        if (accounts.isEmpty()) ui.add(list, ui.text("服务器暂无同平台账号", 13, R.color.farm_muted), 8);
        for (ApiClient.Account account : accounts)
            ui.button(list, account.selectionLabel(), Ui.Tone.TONAL, () -> showConfirmation(account, false));
        int height = accounts.isEmpty() ? ui.dp(40) : Math.min(ui.dp(220), ui.dp(accounts.size() * 64));
        body.addView(scroll, new LinearLayout.LayoutParams(-1, height));
        if (sendCode) ui.button(body, "新增账号", Ui.Tone.TEXT, this::showCreate);
        ui.button(body, "返回", Ui.Tone.TEXT, this::showResults);
    }

    private void showConfirmation(ApiClient.Account account, boolean detected) {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); ui.add(body, ui.strong(detected ? "已匹配账号" : "确认同步", 16), 8);
        ui.add(body, ui.text(account.selectionLabel(), 14, R.color.farm_ink), 8);
        ui.add(body, ui.text(summary() + (sendCode ? "。该账号的登录将更新为本次 Code。" : "，保留服务器已有好友。"), 13, R.color.farm_muted), 12);
        ui.button(body, "确认同步", Ui.Tone.PRIMARY, () -> submit(account));
        ui.button(body, "选择其他账号", Ui.Tone.TEXT, this::showAccounts);
        ui.button(body, "取消", Ui.Tone.TEXT, this::showResults);
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
                main.post(() -> finish("同步完成，VPN 继续获取"));
            } catch (Exception error) { main.post(() -> fail(error)); }
        });
    }

    private void showCreate() {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); ui.add(body, ui.strong("新增账号", 16), 8);
        ui.add(body, ui.text(summary() + "；备注会附加手机标识，避免覆盖同名账号。", 12, R.color.farm_muted), 8);
        TextInputEditText name = ui.input(body, "账号备注", "", false);
        ui.button(body, "确认新增并同步", Ui.Tone.PRIMARY, () -> {
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
                try { api.create(base, actual, snapshot, gids); main.post(() -> finish("新账号已同步，VPN 继续获取")); }
                catch (Exception error) { main.post(() -> fail(error)); }
            });
        });
        ui.button(body, "返回", Ui.Tone.TEXT, this::showAccounts);
        formInput = true; setEditing(!collapsed);
    }

    private void showProgress() {
        resetBody(); ui.add(body, ui.strong("正在同步…", 16), 8);
        ui.add(body, ui.text("完成后可以继续获取，也可以清除结果后换号。", 13, R.color.farm_muted), 8);
    }

    private void finish(String text) {
        busy = false; message = text;
        if (selected != null) messageGeneration = selected.generation();
        if (!closed && panel != null) showResults();
        AppState.changed();
    }

    private void fail(Exception error) {
        busy = false;
        message = error instanceof java.io.IOException || error instanceof IllegalArgumentException
                ? error.getMessage() : "操作未完成，请检查服务器设置后重试";
        if (message == null || message.isBlank()) message = "同步未完成，请重试";
        if (selected != null) messageGeneration = selected.generation();
        if (!closed && panel != null) showResults();
    }

    private void stale() { selected = null; message = "结果或服务器配置已变更，请重新选择"; showResults(); }

    private void setEditing(boolean value) {
        if (!value && editing && panel != null)
            context.getSystemService(InputMethodManager.class).hideSoftInputFromWindow(panel.getWindowToken(), 0);
        editing = value;
        if (layout != null) { layout.flags = windowFlags(value); updateWindow(); }
    }

    private void updateWindow() {
        if (panel == null) return;
        LinearLayout current = panel;
        try { windows.updateViewLayout(panel, layout); }
        catch (RuntimeException revoked) {
            // Finish this render before clearing its view references.
            main.post(() -> { if (panel == current) removeWindow(); });
        }
    }

    private void removeWindow() {
        if (panel == null) return;
        if (editing) context.getSystemService(InputMethodManager.class).hideSoftInputFromWindow(panel.getWindowToken(), 0);
        LinearLayout removed = panel; panel = null; body = null; result = null; detail = null; editing = false;
        try { windows.removeViewImmediate(removed); } catch (RuntimeException alreadyRemoved) { /* Permission may have been revoked. */ }
    }

    public void close() {
        closed = true; AppState.unlisten(listener); main.removeCallbacks(tick); removeWindow();
    }
}
