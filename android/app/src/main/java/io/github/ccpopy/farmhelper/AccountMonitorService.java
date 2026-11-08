package io.github.ccpopy.farmhelper;

import android.app.Service;
import android.content.Intent;
import android.os.IBinder;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/** One watch per server: a slow, offline or unauthorized server never delays alerts from the others. */
public final class AccountMonitorService extends Service {
    private final Map<String, Watch> watches = new ConcurrentHashMap<>();
    private volatile boolean active;

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || "STOP".equals(intent.getAction())) {
            // The notification's stop action turns the feature off, matching the in-app switch.
            if (intent != null) AppState.settings.monitorEnabled(false);
            stopSelf(); return START_NOT_STICKY;
        }
        int count = AppState.settings.monitoredCount();
        startForeground(12, Notifications.ongoing(this, AccountMonitorService.class, "掉线提醒已开启",
                "正在提醒 " + count + " 个账号", 12));
        if (count == 0) { stopSelf(); return START_NOT_STICKY; }
        if (!active) {
            active = true; AppState.monitoring = true; AppState.monitorDetail = "正在首次检查"; AppState.monitorLastCheckedAt = 0;
            AppState.log("通知", "开始监听服务器下线事件；旧版服务器回退为定时检查，不依赖 VPN");
        }
        reconcile(); AppState.changed();
        return START_NOT_STICKY;
    }

    /** Start watches for newly selected servers; retire those removed or re-keyed. */
    private synchronized void reconcile() {
        if (!active) return;
        Map<String, SettingsStore> wanted = new HashMap<>();
        for (SettingsStore server : AppState.settings.readyServers())
            if (!server.monitoredIds().isEmpty()) wanted.put(server.id(), server);
        for (Watch watch : watches.values()) {
            SettingsStore server = wanted.get(watch.id);
            // Retire quietly here; the stop decision is made once below, after new watches exist.
            if (server == null || !server.connectionId().equals(watch.connection)) { watch.close(); watches.remove(watch.id, watch); }
        }
        for (SettingsStore server : wanted.values()) {
            if (watches.containsKey(server.id())) continue;
            Watch watch = new Watch(server); watches.put(server.id(), watch);
            watch.scheduler.execute(watch::poll);
        }
        if (running() == 0) stopSelf();
    }
    private int running() {
        int count = 0;
        for (Watch watch : watches.values()) if (!watch.stopped && !watch.authFailed) count++;
        return count;
    }

    private final class Watch {
        final String id, connection;
        // Long polls occupy a thread. Each selected server needs its own lane, including the fourth.
        final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
        final AccountTransitions transitions = new AccountTransitions();
        final Map<String, String> delivered = new HashMap<>();
        Set<String> relaySelection = java.util.Collections.emptySet();
        String relayCursor = "";
        int failures;
        boolean relayUnavailable;
        volatile boolean stopped, authFailed;
        Watch(SettingsStore server) { id = server.id(); connection = server.connectionId(); }

        SettingsStore server() {
            SettingsStore server = AppState.settings.server(id);
            return server != null && server.ready() && server.connectionId().equals(connection) ? server : null;
        }
        void close() { stopped = true; scheduler.shutdownNow(); }
        void end() {
            synchronized (AccountMonitorService.this) {
                close(); watches.remove(id, this);
                if (active && running() == 0) stopSelf();
            }
        }
        void poll() {
            if (!active || stopped) return;
            SettingsStore server = server();
            if (server == null) { end(); return; }
            try {
                Set<String> selected = server.monitoredIds();
                if (selected.isEmpty()) { end(); return; }
                transitions.retain(selected);
                ApiClient client = new ApiClient(server);
                if (!relayUnavailable) {
                    try { pollRelay(server, client, selected); return; }
                    catch (ApiClient.RelayUnsupportedException unsupported) {
                        relayUnavailable = true;
                        AppState.log("通知", "服务器尚未提供手机中转，使用原有定时检查");
                    }
                }
                List<ApiClient.Account> accounts = client.accounts();
                Map<String, ApiClient.Account> index = new HashMap<>();
                for (ApiClient.Account account : accounts) index.put(account.id, account);
                Map<String, Boolean> observations = new HashMap<>();
                for (String accountId : selected) {
                    if (!active || stopped) return;
                    ApiClient.Account account = index.get(accountId);
                    observations.put(accountId, account != null && client.connected(account));
                }
                if (!active || stopped || !client.isCurrent()) { end(); return; }
                if (!selected.equals(server.monitoredIds())) { transitions.retain(server.monitoredIds()); return; }
                // Apply a cycle atomically only after every selected account was checked.
                // A partial cycle/network error must not leave an offline streak behind.
                for (String accountId : selected) {
                    ApiClient.Account account = index.get(accountId);
                    if (transitions.observe(accountId, observations.get(accountId))) {
                        Notifications.alert(AccountMonitorService.this, server, accountId, account == null ? "" : account.uuid,
                                (account == null ? "服务器账号" : account.name) + " 已离线",
                                server.label() + " · " + (account == null ? "账号已从服务器列表移除，请在管理页面确认。"
                                        : "连续两次未连接。可能是登录失效、手动停止或断线；点此重新获取 Code。"));
                        AppState.log("通知", "已提醒一个离线账号；通知已绑定服务器账号 ID");
                    }
                }
                if (failures > 0) AppState.log("通知", "服务器连接已恢复");
                failures = 0;
                AppState.monitorLastCheckedAt = System.currentTimeMillis();
                AppState.monitorDetail = "检查成功；等待下次检查"; AppState.changed();
            } catch (ApiClient.AuthException invalidToken) {
                if (!active || stopped) return;
                transitions.interrupted(); authFailed = true;
                AppState.monitorDetail = server.label() + " 的 Token 已失效，该服务器的提醒已暂停";
                AppState.log("通知", "服务器认证失败，已暂停该服务器的检查；请更新管理 Token");
                Notifications.serverAuth(AccountMonitorService.this, server); AppState.changed();
                scheduler.shutdown();
                synchronized (AccountMonitorService.this) { if (active && running() == 0) stopSelf(); }
            } catch (Exception e) {
                if (!active || stopped) return;
                transitions.interrupted();
                failures = Math.min(5, failures + 1);
                if (failures == 1) AppState.log("通知", "检查失败，已退避重试；不会把网络故障当作账号下线");
                AppState.monitorDetail = "无法完成检查；网络或服务器异常，稍后重试"; AppState.changed();
            } finally {
                if (active && !stopped && !authFailed && !scheduler.isShutdown()) {
                    long interval = AppState.settings.pollSeconds();
                    long delay = !relayUnavailable && failures == 0 ? 0 : interval + Math.min(900L, interval * ((1L << failures) - 1));
                    try { scheduler.schedule(this::poll, delay, TimeUnit.SECONDS); }
                    catch (java.util.concurrent.RejectedExecutionException stoppedDuringSchedule) { /* The watch was retired. */ }
                }
            }
        }
        private void pollRelay(SettingsStore server, ApiClient client, Set<String> selected) throws Exception {
            if (!relaySelection.equals(selected)) {
                relaySelection = new java.util.HashSet<>(selected); relayCursor = ""; delivered.keySet().retainAll(selected);
            }
            ApiClient.OfflineBatch batch = client.offlineEvents(relayCursor, selected);
            if (!active || stopped || !client.isCurrent()) return;
            if (!selected.equals(server.monitoredIds())) { relayCursor = ""; return; }
            Map<String, ApiClient.Account> index = new HashMap<>();
            if (!batch.events().isEmpty()) for (ApiClient.Account account : client.accounts()) index.put(account.id, account);
            java.util.List<ApiClient.OfflineEvent> pending = new java.util.ArrayList<>();
            for (ApiClient.OfflineEvent event : batch.events()) {
                ApiClient.Account account = index.get(event.accountId());
                // IDs may be reused after deletion. Only the same stable identity may open the relogin flow.
                if (account == null || !account.uuid.equals(event.accountUuid()) || !account.platform.equals(event.platform())
                        || event.eventId().equals(delivered.get(account.id))) continue;
                if (!client.connected(account)) pending.add(event);
            }
            if (!active || stopped || !client.isCurrent() || !selected.equals(server.monitoredIds())) return;
            for (ApiClient.OfflineEvent event : pending) {
                ApiClient.Account account = index.get(event.accountId());
                Notifications.alert(AccountMonitorService.this, server, account.id, account.uuid, account.name + " 需要重新登录",
                        server.label() + " · 服务器检测到账号掉线。点此打开对应账号，重新扫码或获取 Code 后同步。");
                delivered.put(account.id, event.eventId());
                AppState.log("通知", "已收到服务器下线事件，通知绑定对应账号");
            }
            relayCursor = batch.cursor(); failures = 0;
            AppState.monitorLastCheckedAt = System.currentTimeMillis();
            AppState.monitorDetail = "服务器中转已连接，等待账号下线事件"; AppState.changed();
        }
    }

    @Override public synchronized void onDestroy() {
        active = false;
        for (Watch watch : watches.values()) watch.close();
        watches.clear();
        AppState.monitoring = false; AppState.changed();
        stopForeground(STOP_FOREGROUND_REMOVE);
        AppState.log("通知", "账号通知已停止；不会自行重启");
        super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}
