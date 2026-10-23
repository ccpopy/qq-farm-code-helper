package io.github.ccpopy.farmhelper;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.text.InputType;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;
import com.google.android.material.button.MaterialButton;
import com.google.android.material.materialswitch.MaterialSwitch;
import com.google.android.material.textfield.TextInputEditText;
import com.google.android.material.textfield.TextInputLayout;

/** One visual vocabulary for every screen: grouped cards, list rows, pill buttons. */
final class Ui {
    static final Typeface MEDIUM = Typeface.create("sans-serif-medium", Typeface.NORMAL);
    enum Tone { PRIMARY, TONAL, TEXT, DANGER }
    private final Context context;
    Ui(Context context) { this.context = context; }

    int dp(float value) { return Math.round(value * context.getResources().getDisplayMetrics().density); }
    int color(int res) { return context.getColor(res); }

    TextView text(String value, float sp, int colorRes) {
        TextView view = new TextView(context); view.setText(value); view.setTextSize(sp);
        view.setTextColor(color(colorRes)); view.setLineSpacing(dp(2), 1); return view;
    }
    TextView strong(String value, float sp) { TextView view = text(value, sp, R.color.farm_ink); view.setTypeface(MEDIUM); return view; }
    LinearLayout column() { LinearLayout layout = new LinearLayout(context); layout.setOrientation(LinearLayout.VERTICAL); return layout; }
    LinearLayout row() {
        LinearLayout layout = new LinearLayout(context); layout.setOrientation(LinearLayout.HORIZONTAL);
        layout.setGravity(Gravity.CENTER_VERTICAL); return layout;
    }
    void add(LinearLayout parent, View view, int bottomDp) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2); params.bottomMargin = dp(bottomDp);
        parent.addView(view, params);
    }
    GradientDrawable shape(int colorRes, float radiusDp) {
        GradientDrawable shape = new GradientDrawable(); shape.setColor(color(colorRes)); shape.setCornerRadius(dp(radiusDp)); return shape;
    }
    GradientDrawable oval(int colorRes) {
        GradientDrawable shape = new GradientDrawable(); shape.setShape(GradientDrawable.OVAL); shape.setColor(color(colorRes)); return shape;
    }

    /** Small muted label above a grouped card. */
    void section(LinearLayout parent, String label) {
        TextView view = text(label, 13, R.color.farm_muted); view.setTypeface(MEDIUM);
        view.setPadding(dp(6), dp(14), dp(6), dp(8)); parent.addView(view);
    }
    /** Rounded surface whose rows are separated by hairlines. */
    LinearLayout card(LinearLayout parent) {
        LinearLayout card = column(); card.setBackground(shape(R.color.farm_card, 12)); card.setClipToOutline(true);
        add(parent, card, 4); return card;
    }
    LinearLayout padded(LinearLayout card) {
        LinearLayout body = column(); body.setPadding(dp(16), dp(16), dp(16), dp(12)); card.addView(body); return body;
    }
    /** A list row: title, optional subtitle, optional trailing control. Returns the subtitle view. */
    TextView listRow(LinearLayout card, String title, String subtitle, View trailing, Runnable click) {
        return listRow(card, null, title, subtitle, trailing, click);
    }
    TextView listRow(LinearLayout card, View leading, String title, String subtitle, View trailing, Runnable click) {
        if (card.getChildCount() > 0) divider(card);
        LinearLayout row = row(); row.setMinimumHeight(dp(56)); row.setPadding(dp(16), dp(10), dp(12), dp(10));
        if (leading != null) {
            LinearLayout.LayoutParams params = params(leading); params.setMarginEnd(dp(12));
            row.addView(leading, params);
        }
        LinearLayout texts = column();
        texts.addView(text(title, 15, R.color.farm_ink));
        TextView sub = text(subtitle == null ? "" : subtitle, 12, R.color.farm_muted);
        sub.setVisibility(subtitle == null || subtitle.isEmpty() ? View.GONE : View.VISIBLE); texts.addView(sub);
        row.addView(texts, new LinearLayout.LayoutParams(0, -2, 1));
        if (trailing == null && click != null) trailing = icon(R.drawable.ic_chevron, R.color.farm_line_strong, 20);
        if (trailing != null) {
            LinearLayout.LayoutParams params = params(trailing); params.setMarginStart(dp(8));
            row.addView(trailing, params);
        }
        if (click != null) { ripple(row); row.setOnClickListener(v -> click.run()); }
        card.addView(row, new LinearLayout.LayoutParams(-1, -2)); return sub;
    }
    private static LinearLayout.LayoutParams params(View view) {
        return view.getLayoutParams() instanceof LinearLayout.LayoutParams ? (LinearLayout.LayoutParams) view.getLayoutParams() : new LinearLayout.LayoutParams(-2, -2);
    }
    /** A switch row; tapping anywhere on the row toggles it. */
    MaterialSwitch switchRow(LinearLayout card, String title, String subtitle, boolean checked, java.util.function.Consumer<Boolean> changed) {
        MaterialSwitch toggle = new MaterialSwitch(context); toggle.setChecked(checked);
        toggle.setOnCheckedChangeListener((button, value) -> { if (button.getTag() != QUIET) changed.accept(value); });
        listRow(card, title, subtitle, toggle, toggle::toggle);
        toggle.setContentDescription(title);
        return toggle;
    }
    private static final Object QUIET = new Object();
    /** Reflect state without reporting it as a user change. */
    static void setQuietly(MaterialSwitch toggle, boolean checked) {
        if (toggle.isChecked() == checked) return;
        toggle.setTag(QUIET); toggle.setChecked(checked); toggle.setTag(null);
    }
    void divider(LinearLayout card) {
        View line = new View(context); line.setBackgroundColor(color(R.color.farm_line));
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, Math.max(1, dp(.6f))); params.setMarginStart(dp(16));
        card.addView(line, params);
    }
    ImageView icon(int drawable, int tintRes, int sizeDp) {
        ImageView view = new ImageView(context); view.setImageResource(drawable);
        view.setImageTintList(ColorStateList.valueOf(color(tintRes)));
        view.setLayoutParams(new LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp))); return view;
    }
    /** 48dp touch target around a 24dp glyph. */
    ImageView iconButton(int drawable, int tintRes, String description, Runnable click) {
        return iconButton(drawable, tintRes, 48, description, click);
    }
    ImageView iconButton(int drawable, int tintRes, int sizeDp, String description, Runnable click) {
        int inset = dp((sizeDp - 20) / 2f);
        ImageView view = icon(drawable, tintRes, sizeDp); view.setPadding(inset, inset, inset, inset);
        view.setContentDescription(description); view.setFocusable(true);
        TypedValue value = new TypedValue();
        context.getTheme().resolveAttribute(android.R.attr.selectableItemBackgroundBorderless, value, true);
        view.setBackgroundResource(value.resourceId); view.setOnClickListener(v -> click.run()); return view;
    }
    void ripple(View view) {
        TypedValue value = new TypedValue();
        context.getTheme().resolveAttribute(android.R.attr.selectableItemBackground, value, true);
        view.setForeground(androidx.appcompat.content.res.AppCompatResources.getDrawable(context, value.resourceId)); view.setFocusable(true); view.setClickable(true);
    }
    /**
     * PRIMARY / TONAL are full-size actions. TEXT and DANGER are compact pills; they still carry a
     * filled shape so that every tappable control reads as a button, never as plain text.
     */
    MaterialButton button(String label, Tone tone, Runnable action) {
        boolean compact = tone == Tone.TEXT || tone == Tone.DANGER;
        MaterialButton view = new MaterialButton(context);
        view.setText(label); view.setAllCaps(false); view.setTextSize(compact ? 14 : 15); view.setLetterSpacing(0);
        view.setCornerRadius(dp(compact ? 8 : 10)); view.setMinHeight(dp(compact ? 40 : 52)); view.setMinimumHeight(dp(compact ? 40 : 52));
        view.setInsetTop(0); view.setInsetBottom(0); view.setElevation(0); view.setStateListAnimator(null);
        if (compact) view.setPadding(dp(16), 0, dp(16), 0);
        int background = tone == Tone.PRIMARY ? R.color.farm_accent : tone == Tone.DANGER ? R.color.farm_danger_tint : R.color.farm_tint;
        int foreground = tone == Tone.PRIMARY ? R.color.farm_on_accent : tone == Tone.DANGER ? R.color.farm_danger : R.color.farm_accent;
        int[][] states = {{-android.R.attr.state_enabled}, {}};
        int fill = color(background);
        view.setBackgroundTintList(new ColorStateList(states, new int[]{(fill & 0x00FFFFFF) | 0x55000000, fill}));
        int ink = color(foreground);
        ColorStateList inks = new ColorStateList(states, new int[]{(ink & 0x00FFFFFF) | 0x66000000, ink});
        view.setTextColor(inks); view.setIconTint(inks);
        view.setRippleColor(ColorStateList.valueOf((ink & 0x00FFFFFF) | 0x22000000));
        view.setOnClickListener(v -> action.run()); return view;
    }
    MaterialButton button(LinearLayout parent, String label, Tone tone, Runnable action) {
        MaterialButton view = button(label, tone, action);
        if (parent.getOrientation() == LinearLayout.HORIZONTAL) {
            boolean compact = tone == Tone.TEXT || tone == Tone.DANGER;
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(compact ? -2 : 0, -2, compact ? 0 : 1);
            if (parent.getChildCount() > 0) params.setMarginStart(dp(8));
            parent.addView(view, params);
        } else add(parent, view, 8);
        return view;
    }
    /** Give every child of a row the same width, so a button pair reads as one control. */
    void equalize(LinearLayout row) {
        for (int i = 0; i < row.getChildCount(); i++) {
            View child = row.getChildAt(i);
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, -2, 1);
            if (i > 0) params.setMarginStart(dp(8));
            child.setLayoutParams(params);
        }
    }
    /** A value over its label on a tinted tile; the same shape the app uses for capture results. */
    LinearLayout statTile(LinearLayout parent, String value, String label) {
        LinearLayout cell = column(); cell.setBackground(shape(R.color.farm_surface, 8));
        cell.setPadding(dp(12), dp(8), dp(12), dp(8));
        TextView top = strong(value, 15); top.setSingleLine(true); cell.addView(top);
        cell.addView(text(label, 11, R.color.farm_muted));
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, -2, 1);
        if (parent.getChildCount() > 0) params.setMarginStart(dp(8));
        parent.addView(cell, params); return cell;
    }
    MaterialButton button(LinearLayout parent, String label, Tone tone, int icon, Runnable action) {
        MaterialButton view = button(parent, label, tone, action); withIcon(view, icon); return view;
    }
    void withIcon(MaterialButton view, int icon) {
        view.setIconResource(icon); view.setIconSize(dp(18)); view.setIconPadding(dp(6));
        view.setIconGravity(MaterialButton.ICON_GRAVITY_TEXT_START);
    }
    /** Screen capture follows the user's privacy setting; blocked (black) by default. */
    static void secure(android.view.Window window) {
        if (window == null) return;
        if (AppState.settings.allowScreenshots()) window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE);
        else window.addFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE);
    }
    View dot(int colorRes) {
        View dot = new View(context); dot.setBackground(oval(colorRes));
        dot.setLayoutParams(new LinearLayout.LayoutParams(dp(10), dp(10))); return dot;
    }
    TextInputEditText input(LinearLayout parent, String label, String value, boolean secret) {
        TextInputLayout layout = new TextInputLayout(context); layout.setHint(label);
        layout.setBoxBackgroundMode(TextInputLayout.BOX_BACKGROUND_OUTLINE);
        float r = dp(8); layout.setBoxCornerRadii(r, r, r, r);
        layout.setBoxStrokeColor(color(R.color.farm_accent)); layout.setHintTextColor(ColorStateList.valueOf(color(R.color.farm_accent)));
        if (secret) layout.setEndIconMode(TextInputLayout.END_ICON_PASSWORD_TOGGLE);
        TextInputEditText edit = new TextInputEditText(layout.getContext()); edit.setTextSize(15); edit.setText(value); edit.setSaveEnabled(false);
        edit.setSingleLine(true);
        edit.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
        edit.setImeOptions(EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
        edit.setInputType(InputType.TYPE_CLASS_TEXT | (secret ? InputType.TYPE_TEXT_VARIATION_PASSWORD : InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS));
        layout.addView(edit, new LinearLayout.LayoutParams(-1, -2)); add(parent, layout, 10); return edit;
    }
    static TextInputLayout field(TextInputEditText edit) { return (TextInputLayout) edit.getParent().getParent(); }
    static String value(TextInputEditText edit) { return edit.getText() == null ? "" : edit.getText().toString().trim(); }
}
