package io.github.ccpopy.farmhelper;

import android.animation.ValueAnimator;
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
import android.view.animation.DecelerateInterpolator;
import android.view.inputmethod.InputMethodManager;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.textfield.TextInputEditText;
import java.util.List;

/**
 * A user-enabled companion window. Collapsed it is a small bubble that docks to a screen edge;
 * expanded it is a compact translucent card. Collapsing and closing are deliberately different
 * gestures: the chevron or a tap outside collapses, dragging the bubble onto the bottom target closes.
 */
public final class CaptureOverlay {
    private static final float BUBBLE_ALPHA = .82f, PANEL_ALPHA = .94f;
    private static final int BUBBLE = 52, PANEL = 256, EDGE = 6;
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
    // Drop target that appears while the bubble is dragged.
    private FrameLayout dropTarget;
    private View dropCircle;
    private WindowManager.LayoutParams dropLayout;
    private boolean overDrop;
    private ValueAnimator docking;
    private boolean closed, attached, collapsed = true, busy, editing, formInput;
    private int x = -1, y = -1;
    private String message = "";
    private long messageGeneration = -1;
    private AppState.Capture selected;
    private List<ServerDirectory.Group> groups = List.of();
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
        if (docking == null) constrain();
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
        boolean lit = AppState.capturing || hasCode || count > 0;

        bubbleIcon.setImageResource(hasCode || count > 0 ? R.drawable.ic_check : AppState.capturing ? R.drawable.ic_power : R.drawable.ic_farm_leaf);
        bubble.setBackground(ui.oval(lit ? R.color.farm_accent : R.color.farm_card));
        bubbleIcon.setImageTintList(android.content.res.ColorStateList.valueOf(ui.color(lit ? R.color.farm_on_accent : R.color.farm_accent)));
        bubbleBadge.setText(count > 0 ? String.valueOf(count) : "");
        bubbleBadge.setVisibility(count > 0 ? View.VISIBLE : View.GONE);

        if (detail != null) {
            statusDot.setBackground(ui.oval(AppState.capturing ? R.color.farm_accent : R.color.farm_line_strong));
            codeValue.setText(hasCode ? "已取得" : AppState.capturing ? "等待中" : "—");
            friendValue.setText(String.valueOf(count));
            detail.setText(busy ? "正在同步…" : !message.isEmpty() ? message : AppState.status);
            power.setText(AppState.capturing ? "停止" : "开始获取");
            power.setEnabled(!busy);
            boolean available = !busy && !ApiClient.isPushing();
            sync.setEnabled(available && ((hasCode && AppState.settings.pushCode()) || (count > 0 && AppState.settings.pushFriends())));
            friends.setVisibility(count > 0 ? View.VISIBLE : View.GONE); friends.setEnabled(available);
            clear.setVisibility(capture != null ? View.VISIBLE : View.GONE); clear.setEnabled(available);
        }
    }

    private int screenWidth() { return context.getResources().getDisplayMetrics().widthPixels; }
    private int screenHeight() { return context.getResources().getDisplayMetrics().heightPixels; }
    /** Keep the window on screen after rotation, resize, or a collapse/expand. */
    private void constrain() {
        int width = width(), height = Math.max(ui.dp(BUBBLE), root.getHeight());
        if (x < 0) { x = screenWidth() - width - ui.dp(EDGE); y = screenHeight() / 3; }
        int boundedX = Math.max(0, Math.min(x, screenWidth() - width));
        int boundedY = Math.max(0, Math.min(y, screenHeight() - height));
        if (layout.width != width || layout.x != boundedX || layout.y != boundedY) {
            layout.width = width; layout.x = x = boundedX; layout.y = y = boundedY; updateWindow();
        }
    }
    private int width() {
        if (collapsed) return ui.dp(BUBBLE + 6);
        return Math.min(ui.dp(PANEL), screenWidth() - ui.dp(24));
    }
    private int windowFlags(boolean focusable) {
        int flags = WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL | WindowManager.LayoutParams.FLAG_WATCH_OUTSIDE_TOUCH;
        if (!focusable) flags |= WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE;
        if (!AppState.settings.allowScreenshots()) flags |= WindowManager.LayoutParams.FLAG_SECURE;
        return flags;
    }

    // ------------------------------------------------------------------ 窗口
    private void createWindow() {
        root = new FrameLayout(context);
        // A tap anywhere outside the open card folds it back into the bubble.
        root.setOnTouchListener((view, event) -> {
            if (event.getActionMasked() == MotionEvent.ACTION_OUTSIDE && !collapsed && selected == null && !formInput && !busy) collapse();
            return false;
        });

        bubble = ui.column(); bubble.setGravity(Gravity.CENTER); bubble.setElevation(ui.dp(6));
        bubbleIcon = new ImageView(context);
        bubble.addView(bubbleIcon, new LinearLayout.LayoutParams(ui.dp(24), ui.dp(24)));
        bubble.setContentDescription("农场助手，点击展开；拖到底部可关闭");
        installDrag(bubble, this::expand, true);
        bubbleFrame = new FrameLayout(context);
        FrameLayout.LayoutParams bubbleParams = new FrameLayout.LayoutParams(ui.dp(BUBBLE), ui.dp(BUBBLE), Gravity.BOTTOM | Gravity.START);
        bubbleFrame.addView(bubble, bubbleParams);
        bubbleBadge = ui.text("", 11, R.color.farm_on_accent); bubbleBadge.setTypeface(Ui.MEDIUM);
        bubbleBadge.setGravity(Gravity.CENTER); bubbleBadge.setBackground(ui.shape(R.color.farm_amber, 9));
        bubbleBadge.setMinWidth(ui.dp(18)); bubbleBadge.setPadding(ui.dp(5), 0, ui.dp(5), 0);
        bubbleBadge.setElevation(ui.dp(7));
        bubbleFrame.addView(bubbleBadge, new FrameLayout.LayoutParams(-2, ui.dp(18), Gravity.END | Gravity.TOP));
        root.addView(bubbleFrame, new FrameLayout.LayoutParams(ui.dp(BUBBLE + 6), ui.dp(BUBBLE + 6)));

        panel = ui.column(); panel.setPadding(ui.dp(12), ui.dp(6), ui.dp(8), ui.dp(12));
        panel.setBackground(ui.shape(R.color.farm_card, 14)); panel.setElevation(ui.dp(8));
        panel.setVisibility(View.GONE);
        root.addView(panel, new FrameLayout.LayoutParams(-1, -2));

        LinearLayout header = ui.row();
        statusDot = ui.dot(R.color.farm_line_strong);
        LinearLayout.LayoutParams dotParams = new LinearLayout.LayoutParams(ui.dp(8), ui.dp(8));
        dotParams.setMarginStart(ui.dp(2)); dotParams.setMarginEnd(ui.dp(8)); header.addView(statusDot, dotParams);
        TextView handle = ui.strong("农场助手", 14);
        handle.setMinHeight(ui.dp(36)); handle.setGravity(Gravity.CENTER_VERTICAL);
        handle.setContentDescription("拖动可移动悬浮窗");
        installDrag(handle, null, false);
        header.addView(handle, new LinearLayout.LayoutParams(0, -2, 1));
        // Only one window control: fold back into the bubble. Closing is a separate, deliberate drag.
        ImageView fold = ui.iconButton(R.drawable.ic_expand, R.color.farm_muted, 36, "收起为悬浮球", this::collapse);
        fold.setBackground(ui.oval(R.color.farm_surface));
        LinearLayout.LayoutParams foldParams = new LinearLayout.LayoutParams(ui.dp(32), ui.dp(32));
        fold.setPadding(ui.dp(6), ui.dp(6), ui.dp(6), ui.dp(6));
        header.addView(fold, foldParams);
        panel.addView(header);
        body = ui.column(); body.setPadding(0, ui.dp(4), ui.dp(4), 0); panel.addView(body);

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
    private void expand() {
        if (!collapsed) return;
        collapsed = false;
        // Open towards the screen centre so the card never hangs off the docked edge.
        if (x + ui.dp(BUBBLE) / 2 > screenWidth() / 2) x = screenWidth() - width() - ui.dp(EDGE);
        applyCollapsed();
    }
    private void collapse() {
        if (collapsed) return;
        collapsed = true; showResults(); applyCollapsed(); dock();
    }

    /** Drag moves the window; a tap without movement runs the click action. */
    private void installDrag(View handle, Runnable click, boolean closable) {
        handle.setOnTouchListener(new View.OnTouchListener() {
            float downX, downY; int originX, originY; boolean moved;
            @Override public boolean onTouch(View view, MotionEvent event) {
                if (layout == null) return false;
                switch (event.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        if (docking != null) { docking.cancel(); docking = null; }
                        downX = event.getRawX(); downY = event.getRawY(); originX = layout.x; originY = layout.y;
                        moved = false; return true;
                    case MotionEvent.ACTION_MOVE:
                        float dx = event.getRawX() - downX, dy = event.getRawY() - downY;
                        if (!moved && Math.abs(dx) + Math.abs(dy) > ViewConfiguration.get(context).getScaledTouchSlop()) {
                            moved = true;
                            if (closable) showDropTarget();
                        }
                        if (moved) {
                            layout.x = Math.max(0, Math.min(screenWidth() - layout.width, originX + Math.round(dx)));
                            layout.y = Math.max(0, Math.min(screenHeight() - Math.max(ui.dp(BUBBLE), root.getHeight()), originY + Math.round(dy)));
                            x = layout.x; y = layout.y; updateWindow();
                            if (closable) highlightDrop(overDrop());
                        }
                        return true;
                    case MotionEvent.ACTION_UP:
                        if (!moved) { if (click != null) { view.performClick(); click.run(); } return true; }
                        if (closable) {
                            boolean drop = overDrop(); hideDropTarget();
                            if (drop) { closeByUser(); return true; }
                            dock();
                        }
                        return true;
                    case MotionEvent.ACTION_CANCEL:
                        if (closable) { hideDropTarget(); dock(); }
                        return true;
                    default: return false;
                }
            }
        });
    }

    /** Slide the bubble to the nearest side so it stays out of the way. */
    private void dock() {
        if (!collapsed || layout == null || root == null) return;
        int start = layout.x, width = layout.width;
        int end = start + width / 2 < screenWidth() / 2 ? ui.dp(EDGE) - ui.dp(3) : screenWidth() - width - ui.dp(EDGE) + ui.dp(3);
        end = Math.max(0, Math.min(end, screenWidth() - width));
        if (docking != null) docking.cancel();
        docking = ValueAnimator.ofInt(start, end).setDuration(220);
        docking.setInterpolator(new DecelerateInterpolator());
        docking.addUpdateListener(animation -> {
            if (layout == null || root == null) return;
            layout.x = x = (int) animation.getAnimatedValue(); updateWindow();
        });
        docking.addListener(new android.animation.AnimatorListenerAdapter() {
            @Override public void onAnimationEnd(android.animation.Animator animation) { docking = null; }
        });
        docking.start();
    }

    // ------------------------------------------------------------------ 拖到底部关闭
    private void showDropTarget() {
        if (dropTarget != null) return;
        dropTarget = new FrameLayout(context);
        LinearLayout column = ui.column(); column.setGravity(Gravity.CENTER_HORIZONTAL);
        dropCircle = new FrameLayout(context);
        dropCircle.setBackground(ui.oval(R.color.farm_ink)); dropCircle.setAlpha(.72f);
        ImageView mark = ui.icon(R.drawable.ic_close, R.color.farm_card, 26);
        ((FrameLayout) dropCircle).addView(mark, new FrameLayout.LayoutParams(ui.dp(26), ui.dp(26), Gravity.CENTER));
        column.addView(dropCircle, new LinearLayout.LayoutParams(ui.dp(60), ui.dp(60)));
        TextView label = ui.text("拖到这里关闭", 12, R.color.farm_card);
        label.setShadowLayer(ui.dp(3), 0, ui.dp(1), 0x99000000);
        LinearLayout.LayoutParams labelParams = new LinearLayout.LayoutParams(-2, -2); labelParams.topMargin = ui.dp(6);
        column.addView(label, labelParams);
        dropTarget.addView(column, new FrameLayout.LayoutParams(-2, -2, Gravity.CENTER));
        int width = ui.dp(140), height = ui.dp(100);
        dropLayout = new WindowManager.LayoutParams(width, height, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
                        | (AppState.settings.allowScreenshots() ? 0 : WindowManager.LayoutParams.FLAG_SECURE), PixelFormat.TRANSLUCENT);
        dropLayout.gravity = Gravity.TOP | Gravity.START;
        dropLayout.x = (screenWidth() - width) / 2; dropLayout.y = screenHeight() - height - ui.dp(48);
        dropLayout.setTitle("农场助手关闭区域");
        try { windows.addView(dropTarget, dropLayout); dropTarget.setAlpha(0f); dropTarget.animate().alpha(1f).setDuration(150).start(); }
        catch (RuntimeException denied) { hideDropTarget(); }
        overDrop = false;
    }
    private boolean overDrop() {
        if (dropTarget == null || dropLayout == null || layout == null) return false;
        float bubbleX = layout.x + layout.width / 2f, bubbleY = layout.y + ui.dp(BUBBLE) / 2f;
        float targetX = dropLayout.x + dropLayout.width / 2f, targetY = dropLayout.y + ui.dp(30);
        return Math.hypot(bubbleX - targetX, bubbleY - targetY) < ui.dp(64);
    }
    private void highlightDrop(boolean over) {
        if (dropCircle == null || over == overDrop) return;
        overDrop = over;
        dropCircle.setBackground(ui.oval(over ? R.color.farm_danger : R.color.farm_ink));
        dropCircle.animate().scaleX(over ? 1.2f : 1f).scaleY(over ? 1.2f : 1f).alpha(over ? .95f : .72f).setDuration(120).start();
        if (over) dropCircle.performHapticFeedback(android.view.HapticFeedbackConstants.VIRTUAL_KEY);
    }
    private void hideDropTarget() {
        if (dropTarget == null) return;
        FrameLayout removed = dropTarget; dropTarget = null; dropCircle = null; dropLayout = null; overDrop = false;
        try { windows.removeViewImmediate(removed); } catch (RuntimeException alreadyRemoved) { /* Nothing left to hide. */ }
    }
    private void closeByUser() {
        AppState.settings.overlayEnabled(false); removeWindow(); AppState.changed();
        Toast.makeText(context, "悬浮窗已关闭，可在设置中重新打开", Toast.LENGTH_SHORT).show();
    }

    // ------------------------------------------------------------------ 内容
    private void resetBody() {
        formInput = false; setEditing(false); body.removeAllViews(); codeValue = friendValue = null; detail = null;
    }
    /** Smaller than the app's buttons: the card floats over a game and should stay light. */
    private MaterialButton compact(MaterialButton button, boolean pill) {
        int height = ui.dp(pill ? 30 : 38);
        button.setMinHeight(height); button.setMinimumHeight(height);
        button.setMinWidth(0); button.setMinimumWidth(0);
        button.setCornerRadius(ui.dp(8));
        button.setTextSize(pill ? 12.5f : 14);
        button.setPadding(ui.dp(pill ? 12 : 8), 0, ui.dp(pill ? 12 : 8), 0);
        return button;
    }

    private void showResults() {
        if (body == null) return;
        resetBody(); selected = null;
        LinearLayout stats = ui.row();
        LinearLayout codeTile = ui.statTile(stats, "—", "Code"), friendTile = ui.statTile(stats, "0", "好友");
        for (LinearLayout tile : new LinearLayout[]{codeTile, friendTile}) tile.setPadding(ui.dp(10), ui.dp(6), ui.dp(10), ui.dp(6));
        codeValue = (TextView) codeTile.getChildAt(0); friendValue = (TextView) friendTile.getChildAt(0);
        ui.add(body, stats, 6);
        detail = ui.text("", 12, R.color.farm_muted); detail.setMaxLines(2); ui.add(body, detail, 10);
        LinearLayout actions = ui.row();
        power = compact(ui.button(actions, "开始获取", Ui.Tone.TONAL, this::toggleCapture), false);
        sync = compact(ui.button(actions, "同步", Ui.Tone.PRIMARY, () -> choose(false)), false);
        ui.equalize(actions);
        ui.add(body, actions, 0);
        LinearLayout minor = ui.row(); minor.setGravity(Gravity.END);
        friends = compact(ui.button(minor, "只同步好友", Ui.Tone.TEXT, () -> choose(true)), true);
        clear = compact(ui.button(minor, "清除结果", Ui.Tone.DANGER, () -> {
            if (busy || ApiClient.isPushing()) return;
            AppState.clearCapture(); message = "已清除，可换号继续获取"; messageGeneration = -1; refresh();
        }), true);
        LinearLayout.LayoutParams minorParams = new LinearLayout.LayoutParams(-1, -2); minorParams.topMargin = ui.dp(8);
        body.addView(minor, minorParams);
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
        return selected != null && AppState.isCurrentCapture(selected) && ServerDirectory.allCurrent(groups);
    }

    private void choose(boolean friendsOnly) {
        if (busy || ApiClient.isPushing()) return;
        AppState.Capture capture = AppState.capture();
        if (capture == null) { message = "还没有结果"; refresh(); return; }
        if (!AppState.settings.hasServer()) { message = "请先在助手中添加服务器"; refresh(); return; }
        sendCode = !friendsOnly && AppState.settings.pushCode() && !capture.code().isEmpty();
        sendFriends = !capture.gids().isEmpty() && (friendsOnly || AppState.settings.pushFriends());
        if (!sendCode && !sendFriends) { message = "没有可同步的内容"; refresh(); return; }
        selected = capture; busy = true; refresh();
        AppState.IO.execute(() -> {
            List<ServerDirectory.Group> loaded = ServerDirectory.load(capture.platform());
            main.post(() -> {
                busy = false;
                if (closed || root == null) return;
                groups = loaded;
                if (!selectionCurrent()) { stale(); return; }
                if (loaded.stream().noneMatch(ServerDirectory.Group::ok)) {
                    selected = null; message = loaded.size() == 1 ? loaded.get(0).error() : "所有服务器都无法连接"; showResults(); return;
                }
                ServerDirectory.Choice detected = ServerDirectory.detect(groups, capture);
                if (detected != null) showConfirmation(detected, true); else showAccounts();
            });
        });
    }

    private String summary() {
        return (sendCode ? "更新 Code" : "") + (sendCode && sendFriends ? " · " : "")
                + (sendFriends ? "好友 " + selected.gids().size() + " 位" : "");
    }
    private void title(String text, String sub) {
        ui.add(body, ui.strong(text, 15), 2);
        ui.add(body, ui.text(sub, 12, R.color.farm_muted), 8);
    }

    /** Accounts grouped by server; a server heading appears only when there is more than one. */
    private void showAccounts() {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); title("同步到", summary());
        LinearLayout list = ui.column(); list.setBackground(ui.shape(R.color.farm_surface, 10)); list.setClipToOutline(true);
        ScrollView scroll = new ScrollView(context); scroll.addView(list);
        boolean grouped = groups.size() > 1;
        int rows = 0;
        for (ServerDirectory.Group group : groups) {
            if (grouped) {
                TextView heading = ui.text(group.server().label(), 11, R.color.farm_accent); heading.setTypeface(Ui.MEDIUM);
                heading.setPadding(ui.dp(12), ui.dp(10), ui.dp(12), ui.dp(2)); list.addView(heading);
            }
            if (!group.ok()) { list.addView(row(group.error(), "暂时无法同步到这里", R.color.farm_danger, null)); rows++; continue; }
            for (ApiClient.Account account : group.accounts()) {
                list.addView(row(account.name, account.identityLabel(), R.color.farm_ink,
                        () -> showConfirmation(new ServerDirectory.Choice(group, account), false)));
                rows++;
            }
            if (sendCode) { list.addView(row("＋ 新增账号", null, R.color.farm_accent, () -> showCreate(group))); rows++; }
            else if (group.accounts().isEmpty()) { list.addView(row("没有同平台账号", null, R.color.farm_muted, null)); rows++; }
        }
        int height = Math.min(ui.dp(208), ui.dp(rows * 48 + (grouped ? groups.size() * 24 : 0)));
        body.addView(scroll, new LinearLayout.LayoutParams(-1, height));
        LinearLayout actions = ui.row(); actions.setGravity(Gravity.END);
        compact(ui.button(actions, "返回", Ui.Tone.TEXT, this::showResults), true);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.topMargin = ui.dp(8);
        body.addView(actions, params);
    }
    private View row(String name, String sub, int color, Runnable click) {
        LinearLayout row = ui.column(); row.setPadding(ui.dp(12), ui.dp(7), ui.dp(12), ui.dp(7)); row.setMinimumHeight(ui.dp(44));
        row.setGravity(Gravity.CENTER_VERTICAL);
        TextView title = ui.text(name, 14, color); title.setSingleLine(true); row.addView(title);
        if (sub != null) { TextView line = ui.text(sub, 11, R.color.farm_muted); line.setSingleLine(true); row.addView(line); }
        if (click != null) { ui.ripple(row); row.setOnClickListener(v -> click.run()); }
        return row;
    }

    private void showConfirmation(ServerDirectory.Choice choice, boolean detected) {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody(); title(detected ? "已匹配账号" : "确认同步", summary() + (sendCode ? " · 替换该账号的登录" : " · 保留已有好友"));
        LinearLayout card = ui.column(); card.setBackground(ui.shape(R.color.farm_surface, 10));
        card.setPadding(ui.dp(12), ui.dp(8), ui.dp(12), ui.dp(8));
        card.addView(ui.strong(choice.account().name, 14));
        String where = groups.size() > 1 ? choice.group().server().label() + " · " : "";
        card.addView(ui.text(where + choice.account().identityLabel(), 11, R.color.farm_muted));
        ui.add(body, card, 10);
        LinearLayout actions = ui.row();
        compact(ui.button(actions, "换账号", Ui.Tone.TONAL, this::showAccounts), false);
        compact(ui.button(actions, "确认", Ui.Tone.PRIMARY, () -> submit(choice)), false);
        ui.equalize(actions); ui.add(body, actions, 0);
        LinearLayout minor = ui.row(); minor.setGravity(Gravity.END);
        compact(ui.button(minor, "取消", Ui.Tone.TEXT, this::showResults), true);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.topMargin = ui.dp(8);
        body.addView(minor, params);
    }

    private void submit(ServerDirectory.Choice choice) {
        if (busy || ApiClient.isPushing()) return;
        if (!selectionCurrent()) { stale(); return; }
        AppState.Capture snapshot = selected;
        ApiClient api = choice.group().client();
        boolean code = sendCode, gids = sendFriends;
        busy = true; showProgress();
        AppState.IO.execute(() -> {
            try {
                api.push(choice.account().id, snapshot, code, gids);
                if (api.isCurrent()) choice.group().server().target(choice.account());
                main.post(() -> finish("已同步到 " + AccountPicker.describe(choice.group(), choice.account())));
            } catch (Exception error) { main.post(() -> fail(error)); }
        });
    }

    private void showCreate(ServerDirectory.Group group) {
        if (body == null || !selectionCurrent()) { stale(); return; }
        resetBody();
        title("新增账号", (groups.size() > 1 ? group.server().label() + " · " : "") + summary());
        TextInputEditText name = ui.input(body, "账号备注", "", false);
        LinearLayout actions = ui.row();
        compact(ui.button(actions, "返回", Ui.Tone.TONAL, this::showAccounts), false);
        compact(ui.button(actions, "新增并同步", Ui.Tone.PRIMARY, () -> {
            if (busy || ApiClient.isPushing()) return;
            if (!selectionCurrent()) { stale(); return; }
            String base, actual;
            try {
                base = AccountNamePolicy.base(Ui.value(name));
                actual = AccountNamePolicy.unique(base, java.util.UUID.randomUUID().toString().replace("-", "").substring(0, 12));
            } catch (IllegalArgumentException invalid) { Ui.field(name).setError(invalid.getMessage()); return; }
            AppState.Capture snapshot = selected; ApiClient api = group.client(); boolean gids = sendFriends;
            busy = true; showProgress();
            AppState.IO.execute(() -> {
                try {
                    ApiClient.Account created = api.create(base, actual, snapshot, gids);
                    main.post(() -> finish("已新增 " + AccountPicker.describe(group, created)));
                } catch (Exception error) { main.post(() -> fail(error)); }
            });
        }), false);
        ui.equalize(actions); ui.add(body, actions, 0);
        formInput = true; setEditing(!collapsed);
    }

    private void showProgress() {
        resetBody(); title("正在同步…", "完成后可继续获取，或清除结果换号。");
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
        hideDropTarget();
        if (docking != null) { docking.cancel(); docking = null; }
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
