package io.github.ccpopy.farmhelper;

import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.lifecycle.ViewModelProvider;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.dialog.MaterialAlertDialogBuilder;
import com.google.android.material.textfield.TextInputEditText;
import java.util.ArrayList;
import java.util.List;

public final class WxLoginActivity extends AppCompatActivity {
    private Ui ui;
    private WxLoginModel model;
    private TextView status, detail, destination;
    private View accountRow;
    private ImageView mark;
    private FrameLayout badge;
    private MaterialButton primary, secondary, reset;
    private boolean choosing;
    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ui = new Ui(this);
        Ui.secure(getWindow());
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        model = new ViewModelProvider(this).get(WxLoginModel.class);
        if (savedInstanceState == null && model.phase == WxLoginModel.Phase.IDLE
                && AppState.settings.targetPlatform().equals("wx") && !AppState.settings.targetId().isEmpty()) {
            model.targetId = AppState.settings.targetId(); model.targetLabel = AppState.settings.targetName();
        }
        ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true);
        scroll.setBackgroundColor(getColor(R.color.farm_surface));
        LinearLayout body = ui.column(); scroll.addView(body); setContentView(scroll);
        ViewCompat.setOnApplyWindowInsetsListener(scroll, (view, insets) -> {
            var bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
            var ime = insets.getInsets(WindowInsetsCompat.Type.ime());
            body.setPadding(ui.dp(12) + bars.left, ui.dp(4) + bars.top, ui.dp(20) + bars.right, ui.dp(24) + Math.max(bars.bottom, ime.bottom));
            return WindowInsetsCompat.CONSUMED;
        });
        ViewCompat.requestApplyInsets(scroll);

        LinearLayout bar = ui.row(); bar.setMinimumHeight(ui.dp(64));
        bar.addView(ui.iconButton(R.drawable.ic_back, R.color.farm_ink, "返回", this::finish));
        TextView title = ui.strong("微信扫码登录", 22); title.setAccessibilityHeading(true);
        bar.addView(title, new LinearLayout.LayoutParams(0, -2, 1));
        ui.add(body, bar, 8);
        LinearLayout content = ui.column(); content.setPadding(ui.dp(8), 0, 0, 0); ui.add(body, content, 0);

        LinearLayout card = ui.card(content);
        destination = ui.listRow(card, "登录到", "", null, this::chooseAccount);
        accountRow = (View) destination.getParent().getParent();

        View gap = new View(this); ui.add(content, gap, 12);
        LinearLayout hero = ui.column(); hero.setGravity(Gravity.CENTER_HORIZONTAL);
        android.graphics.drawable.GradientDrawable wash = new android.graphics.drawable.GradientDrawable(
                android.graphics.drawable.GradientDrawable.Orientation.TOP_BOTTOM,
                new int[]{getColor(R.color.farm_hero), getColor(R.color.farm_card)});
        wash.setCornerRadius(ui.dp(28)); hero.setBackground(wash);
        hero.setPadding(ui.dp(20), ui.dp(28), ui.dp(20), ui.dp(24));
        FrameLayout stage = new FrameLayout(this);
        badge = new FrameLayout(this); badge.setBackground(ui.oval(R.color.farm_tint)); badge.setElevation(ui.dp(2));
        mark = new ImageView(this); mark.setImageResource(R.drawable.ic_qr);
        badge.addView(mark, new FrameLayout.LayoutParams(ui.dp(48), ui.dp(48), Gravity.CENTER));
        stage.addView(badge, new FrameLayout.LayoutParams(ui.dp(112), ui.dp(112), Gravity.CENTER));
        hero.addView(stage, new LinearLayout.LayoutParams(-1, ui.dp(128)));
        status = ui.strong("", 20); status.setGravity(Gravity.CENTER); status.setId(R.id.wx_login_status);
        status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE); ui.add(hero, status, 6);
        detail = ui.text("", 14, R.color.farm_muted); detail.setGravity(Gravity.CENTER); ui.add(hero, detail, 0);
        ui.add(content, hero, 16);
        primary = ui.button(content, "", Ui.Tone.PRIMARY, this::primaryAction);
        secondary = ui.button(content, "", Ui.Tone.TONAL, this::secondaryAction);
        LinearLayout links = ui.row(); links.setGravity(Gravity.CENTER);
        reset = ui.button(links, "重新开始", Ui.Tone.TEXT, model::reset);
        ui.add(content, links, 0);
        render();
    }
    @Override protected void onResume() { super.onResume(); Ui.secure(getWindow()); }
    @Override protected void onStart() { super.onStart(); model.listener = this::render; render(); }
    @Override protected void onStop() { model.listener = null; super.onStop(); }
    private void render() {
        if (isFinishing() || isDestroyed() || status == null) return;
        WxLoginModel.Phase phase = model.phase;
        boolean editable = model.canStart();
        accountRow.setEnabled(editable && !choosing); accountRow.setAlpha(editable ? 1f : .6f);
        destination.setText(model.draft != null ? model.draft.name() : model.targetId.isEmpty() ? "新增账号" : model.targetLabel);
        destination.setVisibility(View.VISIBLE);
        status.setText(model.status); detail.setText(model.detail);
        detail.setVisibility(model.detail.isEmpty() ? View.GONE : View.VISIBLE);
        boolean success = phase == WxLoginModel.Phase.SAVED;
        boolean failed = phase == WxLoginModel.Phase.FAILED || phase == WxLoginModel.Phase.UNCERTAIN;
        badge.setBackground(ui.oval(success ? R.color.farm_accent : failed ? R.color.farm_amber_tint : R.color.farm_tint));
        mark.setImageResource(success ? R.drawable.ic_check : R.drawable.ic_qr);
        mark.setImageTintList(android.content.res.ColorStateList.valueOf(getColor(success ? R.color.farm_on_accent : failed ? R.color.farm_amber : R.color.farm_accent)));

        String main; boolean mainEnabled = true; String second = null;
        switch (phase) {
            case PREPARING: main = "正在获取…"; mainEnabled = false; break;
            case WAITING: case SCANNED: main = "打开微信扫一扫"; mainEnabled = model.hasLiveQr(); break;
            case AUTHORIZED: main = "完成登录"; break;
            case FINISHING: main = "正在登录…"; mainEnabled = false; break;
            case SAVED: main = "完成"; second = model.checking ? "正在检查…" : "重新检查"; break;
            case UNCERTAIN: main = "返回"; break;
            default: main = phase == WxLoginModel.Phase.FAILED ? "重新获取二维码" : "获取二维码";
        }
        primary.setText(main); primary.setEnabled(mainEnabled);
        secondary.setVisibility(second == null ? View.GONE : View.VISIBLE);
        if (second != null) { secondary.setText(second); secondary.setEnabled(!model.checking); }
        boolean resettable = phase != WxLoginModel.Phase.IDLE && phase != WxLoginModel.Phase.FINISHING
                && phase != WxLoginModel.Phase.UNCERTAIN && phase != WxLoginModel.Phase.SAVED && phase != WxLoginModel.Phase.FAILED;
        reset.setVisibility(resettable ? View.VISIBLE : View.GONE);
        if (model.launchRequested && getLifecycle().getCurrentState().isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)) {
            model.launchRequested = false; openScanner();
        }
    }
    private void primaryAction() {
        switch (model.phase) {
            case WAITING: case SCANNED: openScanner(); break;
            case AUTHORIZED: confirmLogin(); break;
            case SAVED: case UNCERTAIN: finish(); break;
            case IDLE: case FAILED: start(); break;
            default: break;
        }
    }
    private void secondaryAction() { if (model.phase == WxLoginModel.Phase.SAVED) model.checkFriends(); }
    private void start() {
        if (!model.canStart()) return;
        if (!model.targetId.isEmpty() || !model.baseName.isEmpty()) { model.start(true); return; }
        askName(() -> model.start(true));
    }
    private void askName(Runnable then) {
        LinearLayout form = ui.column(); form.setPadding(ui.dp(24), ui.dp(8), ui.dp(24), 0);
        TextInputEditText name = ui.input(form, "账号备注", model.baseName, false);
        Ui.field(name).setHelperText("会自动附加随机后缀，避免与同名账号混淆");
        var dialog = new MaterialAlertDialogBuilder(this).setTitle("新增微信账号").setView(form)
                .setNegativeButton("取消", null).setPositiveButton("继续", null).create();
        dialog.setOnShowListener(d -> dialog.getButton(-1).setOnClickListener(v -> {
            try { model.baseName = AccountNamePolicy.base(Ui.value(name)); }
            catch (IllegalArgumentException error) { Ui.field(name).setError(error.getMessage()); return; }
            dialog.dismiss(); then.run();
        }));
        dialog.show();
    }
    private void openScanner() {
        boolean opened = WechatScanner.open(this);
        Toast.makeText(this, opened ? "扫描另一台设备上显示的二维码" : "请在微信「+」›「扫一扫」中扫码", Toast.LENGTH_LONG).show();
    }
    private void chooseAccount() {
        if (!model.canStart() || choosing) return;
        choosing = true; render();
        AppState.IO.execute(() -> {
            try {
                ApiClient client = new ApiClient(AppState.settings);
                List<ApiClient.Account> accounts = new ArrayList<>();
                for (ApiClient.Account account : client.accounts()) if (account.platform.equals("wx")) accounts.add(account);
                String[] labels = new String[accounts.size() + 1]; labels[0] = "＋ 新增账号";
                for (int i = 0; i < accounts.size(); i++) labels[i + 1] = accounts.get(i).name + "（ID " + accounts.get(i).id + "）";
                runOnUiThread(() -> {
                    if (isFinishing() || isDestroyed() || !model.canStart() || !client.isCurrent()) return;
                    new MaterialAlertDialogBuilder(this).setTitle("登录到").setItems(labels, (dialog, which) -> {
                        if (!model.canStart() || !client.isCurrent()) return;
                        model.targetId = which == 0 ? "" : accounts.get(which - 1).id;
                        model.targetLabel = which == 0 ? "新增账号" : accounts.get(which - 1).name;
                        if (which == 0) askName(this::render); else render();
                    }).show();
                });
            } catch (Exception error) {
                runOnUiThread(() -> Toast.makeText(this, "无法读取账号，请检查服务器设置", Toast.LENGTH_LONG).show());
            } finally { runOnUiThread(() -> { choosing = false; render(); }); }
        });
    }
    private void confirmLogin() {
        if (model.phase != WxLoginModel.Phase.AUTHORIZED || model.draft == null) return;
        ApiClient.WxDraft selected = model.draft;
        new MaterialAlertDialogBuilder(this).setTitle(selected.id().isEmpty() ? "新增并登录？" : "更新并登录？")
                .setMessage(selected.name() + "\n\n请确认扫码的微信就是这个账号。")
                .setNegativeButton("取消", null).setPositiveButton("确认", (dialog, which) -> {
                    if (model.draft == selected) model.finishLogin();
                }).show();
    }
}
