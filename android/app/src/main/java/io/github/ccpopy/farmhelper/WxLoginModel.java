package io.github.ccpopy.farmhelper;

import android.app.Application;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import androidx.lifecycle.AndroidViewModel;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** The short-lived task survives activity recreation and the trip to WeChat, but is never persisted. */
public final class WxLoginModel extends AndroidViewModel {
    public enum Phase { IDLE, PREPARING, WAITING, SCANNED, AUTHORIZED, FINISHING, SAVED, FAILED, UNCERTAIN }
    public Phase phase = Phase.IDLE;
    public String status = "准备就绪", detail = "";
    public String targetId = "", targetLabel = "新增账号", baseName = "";
    /** Pin the selected profile and its credential generation; never fall back to another server. */
    public String profileId = "", profileConnection = "", profileLabel = "";
    public SettingsStore server() {
        return AppState.settings.server(profileId);
    }
    public void selectServer(SettingsStore server) {
        profileId = server.id(); profileConnection = server.connectionId(); profileLabel = server.label();
    }
    public boolean selectionCurrent() {
        SettingsStore selected = server();
        return selected != null && selected.ready() && !profileConnection.isEmpty()
                && profileConnection.equals(selected.connectionId());
    }
    public final String nonce = java.util.UUID.randomUUID().toString().replace("-", "").substring(0, 12);
    public ApiClient.WxDraft draft;
    public Uri image;
    public boolean launchRequested, checking;
    public Runnable listener;
    private ApiClient client;
    private ApiClient.WxTask task;
    private ApiClient.Account saved;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private volatile int generation;
    private int pollFailures;
    public WxLoginModel(Application application) { super(application); }
    private void changed() { if (listener != null) listener.run(); }
    private void post(int run, Runnable action) { main.post(() -> { if (run == generation) { action.run(); changed(); } }); }
    public boolean canStart() { return phase == Phase.IDLE || phase == Phase.FAILED; }
    public boolean hasLiveQr() { return image != null && task != null && System.currentTimeMillis() < task.expiresAt()
            && (phase == Phase.WAITING || phase == Phase.SCANNED || phase == Phase.AUTHORIZED); }
    public void start(boolean openScanner) {
        if (!canStart()) return;
        if (!selectionCurrent()) {
            status = "所选服务器已删除或设置已变化，请重新选择「登录到」"; detail = ""; changed(); return;
        }
        final SettingsStore selectedServer = server();
        final String selectedConnection = profileConnection;
        final String id = targetId, base = baseName;
        final String name;
        try { name = id.isEmpty() ? AccountNamePolicy.unique(base, nonce) : ""; }
        catch (IllegalArgumentException error) { status = error.getMessage(); changed(); return; }
        cancelTask();
        final int run = ++generation;
        phase = Phase.PREPARING; status = "正在获取二维码…"; detail = ""; image = null; draft = null; pollFailures = 0; changed();
        io.execute(() -> {
            ApiClient api = null; ApiClient.WxTask created = null;
            try {
                api = new ApiClient(selectedServer);
                if (!selectedConnection.equals(api.connectionId)) throw new Exception("服务器配置已变化，请重新选择「登录到」");
                ApiClient.WxDraft selected = api.prepareWxAccount(id, base, name);
                if (run != generation) return;
                created = api.createWxTask();
                byte[] bytes = api.wxQr(created);
                if (run != generation) { discard(api, created); return; }
                if (!api.isCurrent()) throw new Exception("服务器配置已变化，请重新开始");
                Uri uri = WxQrImage.save(getApplication(), bytes);
                if (run != generation) { discard(api, created); return; }
                if (!api.isCurrent()) throw new Exception("服务器配置已变化，请重新开始");
                ApiClient readyClient = api; ApiClient.WxTask readyTask = created;
                main.post(() -> {
                    if (run != generation) { discard(readyClient, readyTask); return; }
                    client = readyClient; task = readyTask; draft = selected; image = uri;
                    phase = Phase.WAITING; status = "等待扫码";
                    detail = "二维码已存入相册。请在另一台设备上显示它，用本机微信扫一扫。";
                    launchRequested = openScanner;
                    schedulePoll(run);
                    changed();
                });
            } catch (Exception error) {
                if (api != null && created != null) discard(api, created);
                fail(run, error);
            }
        });
    }
    private void schedulePoll(int run) { main.postDelayed(() -> poll(run), 1200); }
    private void poll(int run) {
        if (run != generation || (phase != Phase.WAITING && phase != Phase.SCANNED)) return;
        if (System.currentTimeMillis() >= task.expiresAt()) { fail(run, new Exception("二维码已过期，请重新获取；旧图片不能继续登录")); return; }
        final ApiClient api = client; final ApiClient.WxTask current = task;
        io.execute(() -> {
            try {
                String state = api.wxStatus(current);
                post(run, () -> {
                    pollFailures = 0;
                    switch (state) {
                        case "authorized": case "ready_for_code":
                            phase = Phase.AUTHORIZED; status = "已授权";
                            detail = "点「完成登录」保存账号并上线。"; break;
                        case "waiting": case "scanned":
                            phase = state.equals("scanned") ? Phase.SCANNED : Phase.WAITING;
                            status = state.equals("scanned") ? "已扫码，请在微信中确认" : "等待扫码";
                            schedulePoll(run); break;
                        default: phase = Phase.FAILED; status = "二维码已取消或失效，请重新获取"; cancelTask(); break;
                    }
                });
            } catch (ApiClient.TemporaryNetworkException temporary) {
                post(run, () -> {
                    if (System.currentTimeMillis() >= current.expiresAt()) {
                        fail(run, new Exception("二维码已过期，请重新获取；旧图片不能继续登录"));
                        return;
                    }
                    // Only the read-only status check is retried; keep the same task after a network interruption.
                    status = "网络中断，正在重试…";
                    detail = "二维码仍有效，无需重新扫码。";
                    long delay = Math.min(8000L, 2000L << Math.min(pollFailures++, 2));
                    main.postDelayed(() -> poll(run), Math.min(delay, Math.max(1, current.expiresAt() - System.currentTimeMillis())));
                });
            } catch (Exception error) { fail(run, error); }
        });
    }
    public void finishLogin() {
        if (phase != Phase.AUTHORIZED) return;
        if (System.currentTimeMillis() >= task.expiresAt()) { fail(generation, new Exception("二维码已过期，请重新获取")); return; }
        final int run = generation;
        final ApiClient api = client; final ApiClient.WxTask current = task; final ApiClient.WxDraft selected = draft;
        phase = Phase.FINISHING; status = "正在登录…"; detail = ""; changed();
        io.execute(() -> {
            try {
                ApiClient.Account account = api.finishWxLogin(current, selected);
                post(run, () -> {
                    task = null; saved = account; phase = Phase.SAVED;
                    status = "登录成功";
                    checkFriends();
                });
            } catch (ApiClient.UncertainWxSaveException uncertain) {
                post(run, () -> { phase = Phase.UNCERTAIN; status = uncertain.getMessage(); detail = "请先在账号页确认，不会自动重试。"; cancelTask(); });
            } catch (Exception error) { fail(run, error); }
        });
    }
    public void checkFriends() {
        if (phase != Phase.SAVED || checking) return;
        checking = true; detail = "正在检查在线状态与好友…"; changed();
        final int run = generation;
        io.execute(() -> {
            try {
                ApiClient.Account current = null;
                for (ApiClient.Account account : client.accounts()) if (account.id.equals(saved.id) && account.platform.equals("wx")) current = account;
                if (current == null) throw new Exception("服务器账号已不存在");
                if (!client.connected(current)) throw new Exception("账号已保存，尚未上线。稍后点「重新检查」。");
                int count = client.wechatFriendCount(current.id);
                post(run, () -> {
                    checking = false;
                    detail = count > 0 ? "已上线，读取到 " + count + " 个有效好友"
                            : "已上线，但未读取到好友";
                });
            } catch (Exception error) { post(run, () -> { checking = false; detail = safeError(error); }); }
        });
    }
    public void reset() {
        if (phase == Phase.FINISHING || phase == Phase.UNCERTAIN) return;
        ++generation; cancelTask(); image = null; draft = null; saved = null; launchRequested = false; checking = false;
        phase = Phase.IDLE; status = "准备就绪"; detail = ""; changed();
    }
    private void fail(int run, Exception error) {
        post(run, () -> { phase = Phase.FAILED; status = safeError(error); detail = ""; launchRequested = false; cancelTask(); });
    }
    private static String safeError(Exception error) {
        String text = error.getMessage();
        return text == null || text.length() > 240 || text.contains("://") ? "操作未完成，请检查服务器与网络后重新获取" : text;
    }
    private void cancelTask() { if (client != null && task != null) discard(client, task); task = null; }
    private static void discard(ApiClient api, ApiClient.WxTask old) {
        AppState.IO.execute(() -> { try { api.cancelWxTask(old); } catch (Exception ignored) { /* Server expires abandoned tasks. */ } });
    }
    @Override protected void onCleared() {
        ++generation; listener = null; main.removeCallbacksAndMessages(null); cancelTask(); io.shutdownNow();
    }
}
