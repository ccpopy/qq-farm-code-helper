package io.github.ccpopy.farmhelper;

import android.app.Service;
import android.content.Intent;
import android.os.IBinder;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

public final class AccountMonitorService extends Service {
    private ScheduledExecutorService scheduler;
    private final AccountTransitions transitions = new AccountTransitions();
    private int failures;
    private volatile boolean active;
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || "STOP".equals(intent.getAction())) { stopSelf(); return START_NOT_STICKY; }
        if (active) return START_NOT_STICKY;
        Set<String> selected = AppState.settings.monitoredIds();
        if (selected.isEmpty()) { AppState.log("通知", "请先选择需要提醒的服务器账号"); stopSelf(); return START_NOT_STICKY; }
        startForeground(12, Notifications.ongoing(this, AccountMonitorService.class, "账号通知已开启",
                "检查 " + selected.size() + " 个账号 · 默认每 " + AppState.settings.pollSeconds() + " 秒", 12));
        active = true; AppState.monitoring = true; AppState.monitorDetail = "正在首次检查"; AppState.monitorLastCheckedAt = 0; AppState.changed();
        AppState.log("通知", "开始独立检查选定账号；连续两次离线才提醒，不依赖 VPN");
        scheduler = Executors.newSingleThreadScheduledExecutor();
        scheduler.execute(this::poll);
        return START_NOT_STICKY;
    }
    private void poll() {
        if (!active) return;
        try {
            Set<String> selected = AppState.settings.monitoredIds();
            if (selected.isEmpty()) { stopSelf(); return; }
            transitions.retain(selected);
            ApiClient client = new ApiClient(AppState.settings);
            List<ApiClient.Account> accounts = client.accounts();
            Map<String, ApiClient.Account> index = new HashMap<>();
            for (ApiClient.Account account : accounts) index.put(account.id, account);
            Map<String, Boolean> observations = new HashMap<>();
            for (String id : selected) {
                if (!active) return;
                ApiClient.Account account = index.get(id);
                observations.put(id, account != null && client.connected(account));
            }
            if (!active || !client.isCurrent()) { stopSelf(); return; }
            // Apply a cycle atomically only after every selected account was checked.
            // A partial cycle/network error must not leave an offline streak behind.
            for (String id : selected) {
                ApiClient.Account account = index.get(id);
                if (transitions.observe(id, observations.get(id))) {
                    Notifications.alert(this, id, (account == null ? "服务器账号" : account.name) + " 已离线",
                            account == null ? "账号已从服务器列表移除，请在管理页面确认。" : "连续两次未连接。可能是登录失效、手动停止或断线；点此重新获取 Code。");
                    AppState.log("通知", "已提醒一个离线账号；通知已绑定服务器账号 ID");
                }
            }
            if (failures > 0) AppState.log("通知", "服务器连接已恢复");
            failures = 0;
            AppState.monitorLastCheckedAt = System.currentTimeMillis();
            AppState.monitorDetail = "检查成功；等待下次检查"; AppState.changed();
        } catch (ApiClient.AuthException invalidToken) {
            if (!active) return;
            transitions.interrupted(); active = false;
            AppState.monitorDetail = "服务器 Token 已失效或权限不足，检查已停止；这不代表 QQ 账号失效";
            AppState.log("通知", "服务器认证失败，已停止检查；请更新管理 Token 后手动开启");
            Notifications.serverAuth(this); AppState.changed(); stopSelf();
        } catch (Exception e) {
            if (!active) return;
            transitions.interrupted();
            failures = Math.min(5, failures + 1);
            if (failures == 1) AppState.log("通知", "检查失败，已退避重试；不会把网络故障当作账号下线");
            AppState.monitorDetail = "无法完成检查；网络或服务器异常，稍后重试，离线计数已重置"; AppState.changed();
        } finally {
            if (active && scheduler != null && !scheduler.isShutdown()) {
                long delay = Math.min(900L, (long)AppState.settings.pollSeconds() * (1L << failures));
                scheduler.schedule(this::poll, delay, TimeUnit.SECONDS);
            }
        }
    }
    @Override public void onDestroy() {
        active = false;
        if (scheduler != null) scheduler.shutdownNow();
        AppState.monitoring = false; AppState.changed();
        stopForeground(STOP_FOREGROUND_REMOVE);
        AppState.log("通知", "账号通知已停止；不会自行重启");
        super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}
