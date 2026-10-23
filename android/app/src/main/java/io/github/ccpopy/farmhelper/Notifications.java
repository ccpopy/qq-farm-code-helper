package io.github.ccpopy.farmhelper;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import androidx.core.app.NotificationCompat;

public final class Notifications {
    public static final String SERVICE = "active-services";
    public static final String ALERT = "account-offline";
    private Notifications() {}
    public static void createChannels(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(SERVICE, "正在运行的功能", NotificationManager.IMPORTANCE_LOW));
        NotificationChannel alerts = new NotificationChannel(ALERT, "账号离线与抓取结果", NotificationManager.IMPORTANCE_HIGH);
        alerts.setDescription("账号连续离线后提醒；点通知选择对应服务器账号重新获取 Code");
        manager.createNotificationChannel(alerts);
    }
    private static PendingIntent open(Context context, String accountId, int request) {
        return open(context, accountId, "", request);
    }
    private static PendingIntent open(Context context, String accountId, String uuid, int request) {
        Intent intent = new Intent(context, MainActivity.class).setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        if (accountId != null && !accountId.isEmpty()) intent.putExtra("accountId", accountId)
                .putExtra("server", AppState.settings.server()).putExtra("connectionId", AppState.settings.connectionId()).putExtra("accountUuid", uuid);
        return PendingIntent.getActivity(context, request, intent, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
    }
    public static Notification ongoing(Context context, Class<?> service, String title, String detail, int id) {
        Intent stop = new Intent(context, service).setAction("STOP");
        PendingIntent action = PendingIntent.getService(context, id, stop, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new NotificationCompat.Builder(context, SERVICE).setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title).setContentText(detail).setOngoing(true).setOnlyAlertOnce(true)
                .setContentIntent(open(context, null, id)).addAction(0, "停止", action).build();
    }
    public static void alert(Context context, String id, String title, String message) {
        alert(context, id, "", title, message);
    }
    public static void alert(Context context, String id, String uuid, String title, String message) {
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            AppState.log("通知", "通知权限未开启，提醒仅显示在应用中"); return;
        }
        int notificationId = 1000 + (id.hashCode() & 0x0fffffff);
        context.getSystemService(NotificationManager.class).notify(notificationId,
                new NotificationCompat.Builder(context, ALERT).setSmallIcon(R.drawable.ic_notification).setContentTitle(title)
                        .setContentText(message).setStyle(new NotificationCompat.BigTextStyle().bigText(message))
                        .setContentIntent(open(context, id, uuid, notificationId)).setAutoCancel(true)
                        .setVisibility(NotificationCompat.VISIBILITY_PRIVATE).build());
    }
    public static void serverAuth(Context context) {
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return;
        Intent intent = new Intent(context, MainActivity.class).setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP).putExtra("showSettings", true);
        PendingIntent pending = PendingIntent.getActivity(context, 901, intent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        context.getSystemService(NotificationManager.class).notify(901,
                new NotificationCompat.Builder(context, ALERT).setSmallIcon(R.drawable.ic_notification)
                        .setContentTitle("服务器管理 Token 需要更新").setContentText("账号检查已停止；不代表 QQ 登录失效。点击更新服务器设置。")
                        .setContentIntent(pending).setAutoCancel(true).setVisibility(NotificationCompat.VISIBILITY_PRIVATE).build());
    }
}
