package io.github.ccpopy.farmhelper;

import android.app.Activity;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import androidx.appcompat.app.AlertDialog;
import com.google.android.material.dialog.MaterialAlertDialogBuilder;
import java.util.List;

/**
 * One list for choosing an account across every saved server. Servers appear as headings only when
 * there is more than one, so a single-server setup looks exactly like a plain account list.
 */
final class AccountPicker {
    interface Pick { void picked(ServerDirectory.Group group, ApiClient.Account account); }
    interface Create { void create(ServerDirectory.Group group); }
    private AccountPicker() {}

    static AlertDialog show(Activity activity, String title, List<ServerDirectory.Group> groups,
                            Pick pick, String createLabel, Create create) {
        Ui ui = new Ui(activity);
        LinearLayout list = ui.column(); list.setPadding(ui.dp(8), ui.dp(4), ui.dp(8), ui.dp(8));
        ScrollView scroll = new ScrollView(activity); scroll.addView(list);
        AlertDialog[] dialog = new AlertDialog[1];
        boolean grouped = groups.size() > 1;
        for (ServerDirectory.Group group : groups) {
            if (grouped) {
                LinearLayout heading = ui.row(); heading.setPadding(ui.dp(16), ui.dp(14), ui.dp(16), ui.dp(4));
                TextView name = ui.text(group.server().label(), 13, R.color.farm_accent); name.setTypeface(Ui.MEDIUM);
                heading.addView(name, new LinearLayout.LayoutParams(0, -2, 1));
                if (!group.server().name().isEmpty()) heading.addView(ui.text(group.server().host(), 12, R.color.farm_muted));
                list.addView(heading);
            }
            LinearLayout rows = ui.column(); list.addView(rows);
            if (!group.ok()) {
                ui.listRow(rows, ui.dot(R.color.farm_danger), group.error(), "这个服务器暂时无法选择", null, null);
                continue;
            }
            for (ApiClient.Account account : group.accounts())
                ui.listRow(rows, account.name, account.identityLabel(), null, () -> { dialog[0].dismiss(); pick.picked(group, account); });
            if (create != null) {
                TextView add = ui.listRow(rows, createLabel, null, ui.icon(R.drawable.ic_add, R.color.farm_accent, 20),
                        () -> { dialog[0].dismiss(); create.create(group); });
                ((TextView) ((LinearLayout) add.getParent()).getChildAt(0)).setTextColor(ui.color(R.color.farm_accent));
            } else if (group.accounts().isEmpty()) {
                TextView empty = ui.text("没有可选的账号", 13, R.color.farm_muted);
                empty.setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(12)); rows.addView(empty);
            }
        }
        dialog[0] = new MaterialAlertDialogBuilder(activity).setTitle(title).setView(scroll)
                .setNegativeButton("取消", null).create();
        Ui.secure(dialog[0].getWindow());
        dialog[0].show();
        return dialog[0];
    }

    /** "服务器 · 账号" when it matters which server, otherwise just the account. */
    static String describe(ServerDirectory.Group group, ApiClient.Account account) {
        boolean many = AppState.settings.readyServers().size() > 1;
        return (many ? group.server().label() + " · " : "") + account.name;
    }
}
