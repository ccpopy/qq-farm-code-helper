package io.github.ccpopy.farmhelper;

import android.app.Activity;
import android.content.Intent;

/** Reuse WeChat's registered launcher shortcut; image selection stays inside WeChat. */
public final class WechatScanner {
    private WechatScanner() {}
    public static boolean open(Activity activity) {
        try {
            activity.startActivity(new Intent("com.tencent.mm.ui.ShortCutDispatchAction")
                    .setPackage("com.tencent.mm")
                    .putExtra("LauncherUI.Shortcut.LaunchType", "launch_type_scan_qrcode"));
            return true;
        } catch (android.content.ActivityNotFoundException | SecurityException unsupported) {
            Intent launch = activity.getPackageManager().getLaunchIntentForPackage("com.tencent.mm");
            if (launch != null) {
                try { activity.startActivity(launch); }
                catch (android.content.ActivityNotFoundException | SecurityException ignored) { /* UI supplies manual directions. */ }
            }
            return false;
        }
    }
}
