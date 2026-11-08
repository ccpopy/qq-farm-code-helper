package io.github.ccpopy.farmhelper;

import android.app.AppOpsManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.provider.Settings;

/**
 * Hosts the companion window independently of the VPN. Keeping it out of {@link CaptureVpnService}
 * is what lets the window survive a stop and start the next capture by itself.
 */
public final class OverlayService extends Service {
    private final Handler main = new Handler(Looper.getMainLooper());
    private CaptureOverlay overlay;
    private AppOpsManager appOps;
    private final Runnable listener = this::check;
    private final AppOpsManager.OnOpChangedListener permissionListener = (op, packageName) -> main.post(this::check);

    /** Show or hide the window to match the current setting; safe to call from anywhere. */
    public static void sync(Context context) {
        boolean wanted = AppState.settings.overlayEnabled() && Settings.canDrawOverlays(context);
        Intent intent = new Intent(context.getApplicationContext(), OverlayService.class).setAction("START");
        try {
            if (wanted) context.getApplicationContext().startForegroundService(intent);
            else context.getApplicationContext().stopService(intent);
        } catch (RuntimeException restricted) { /* The window is optional; never break the caller. */ }
    }

    @Override public void onCreate() {
        super.onCreate();
        AppState.listen(listener);
        appOps = getSystemService(AppOpsManager.class);
        appOps.startWatchingMode(AppOpsManager.OPSTR_SYSTEM_ALERT_WINDOW, getPackageName(), permissionListener);
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || "STOP".equals(intent.getAction())) {
            if (intent != null) {
                AppState.settings.overlayEnabled(false);
                AppState.changed();
            }
            stopSelf();
            return START_NOT_STICKY;
        }
        if (!AppState.settings.overlayEnabled() || !Settings.canDrawOverlays(this)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        if (overlay == null) {
            startForeground(13, Notifications.ongoing(this, OverlayService.class, "农场助手悬浮窗",
                    "查看获取结果并同步；停止只关闭悬浮窗", 13));
            overlay = new CaptureOverlay(this);
        }
        return START_NOT_STICKY;
    }
    private void check() {
        if (!AppState.settings.overlayEnabled() || !Settings.canDrawOverlays(this)) stopSelf();
    }
    @Override public void onDestroy() {
        if (appOps != null) appOps.stopWatchingMode(permissionListener);
        main.removeCallbacksAndMessages(null);
        AppState.unlisten(listener);
        if (overlay != null) { overlay.close(); overlay = null; }
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}
