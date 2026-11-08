package io.github.ccpopy.farmhelper;

import org.json.JSONArray;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/** No redirects: an admin token must never follow a redirect to another host. */
public final class ApiClient {
    private static final java.util.concurrent.atomic.AtomicBoolean PUSHING = new java.util.concurrent.atomic.AtomicBoolean();
    public static boolean isPushing() { return PUSHING.get(); }
    private static void beginPush() throws IOException {
        if (!PUSHING.compareAndSet(false, true)) throw new IOException("正在推送，请等待本次完成");
        AppState.changed();
    }
    private static void endPush() { PUSHING.set(false); AppState.changed(); }
    private void requirePending(AppState.Capture requested, boolean code, boolean friends) throws IOException {
        AppState.Capture current = AppState.capture();
        if (!isCurrent() || current == null || requested == null || current.generation() != requested.generation()
                || current.platform() != requested.platform()) throw new IOException("服务器或本次结果已变化，请重新确认");
        if (code && (requested.code().isEmpty() || !requested.code().equals(current.code())))
            throw new IOException("Code 已推送或过期，请重新获取");
        if (friends && (requested.gids().isEmpty() || !current.gids().containsAll(requested.gids())))
            throw new IOException("好友结果已变化或已同步，请重新确认");
    }
    private final String server;
    private final String token;
    private final SettingsStore settings;
    public final String connectionId;
    public static final class AuthException extends IOException {
        public AuthException() { super("管理 Token 已失效或权限不足，请重新登录服务器并复制 Token"); }
    }
    public static final class RelayUnsupportedException extends IOException {}
    public static final class TemporaryNetworkException extends IOException {
        TemporaryNetworkException() { super("网络连接暂时中断，请稍后重试"); }
    }
    public ApiClient(SettingsStore settings) throws Exception {
        this.settings = settings;
        connectionId = settings.connectionId();
        if (settings.server().isEmpty()) throw new IOException("请先在设置中保存服务器地址与管理 Token");
        server = UrlPolicy.server(settings.server(), settings.allowHttp()).toString();
        token = settings.token();
        if (token.isEmpty()) throw new IOException("请先保存服务器与 Token");
    }
    public boolean isCurrent() { return connectionId.equals(settings.connectionId()); }
    /** The server profile this client talks to. */
    public SettingsStore profile() { return settings; }
    public static final class Account {
        public final String id, name, platform, gid, uuid, qq;
        public final boolean running;
        Account(JSONObject json) {
            id = json.optString("id"); name = json.optString("name", id);
            platform = json.optString("platform", "qq"); gid = AccountIdentity.gid(json.optString("gid"));
            // The panel uses uin; older servers may only expose qq. Neither is a game GID.
            String uin = AccountIdentity.qq(json.optString("uin"));
            qq = platform.equals("qq") ? (uin.isEmpty() ? AccountIdentity.qq(json.optString("qq")) : uin) : "";
            uuid = json.isNull("uuid") ? "" : json.optString("uuid").trim();
            running = json.optBoolean("running");
        }
        public String identityLabel() {
            String source = platform.equals("qq") ? "QQ" : platform.equals("wx") ? "微信" : platform;
            if (!qq.isEmpty()) return "QQ " + qq;
            if (!gid.isEmpty()) return source + " · 农场 GID " + gid;
            return source + " · 账号 ID " + id;
        }
        public String selectionLabel() { return name + "（" + identityLabel() + "）"; }
        public String label() { return selectionLabel() + " · " + (running ? "运行中" : "已停止"); }
    }
    public List<Account> accounts() throws Exception {
        return parseAccounts(call("GET", "/api/accounts", null, null));
    }
    public String knownGid(Account account) {
        return account.gid.isEmpty() ? settings.rememberedGid(connectionId, account) : account.gid;
    }
    public Account matchingAccount(List<Account> accounts, AppState.Capture capture) {
        if (capture.ownGid().isEmpty()) return null;
        Account match = null;
        for (Account account : accounts) {
            if (!account.platform.equals(capture.platform().id) || !capture.ownGid().equals(knownGid(account))) continue;
            if (match != null) return null;
            match = account;
        }
        return match;
    }
    private static List<Account> parseAccounts(JSONObject response) throws Exception {
        Object data = response.opt("data");
        JSONArray array = data instanceof JSONObject ? ((JSONObject)data).optJSONArray("accounts") : (data instanceof JSONArray ? (JSONArray)data : null);
        if (array == null) throw new IOException("服务器未返回可识别的账号列表");
        List<Account> result = new ArrayList<>();
        for (int i = 0; i < array.length(); i++) {
            JSONObject item = array.optJSONObject(i);
            if (item != null && !item.optString("id").isEmpty()) result.add(new Account(item));
        }
        return result;
    }
    public List<Account> checkNewAccount(String baseName, String actualName, AppState.Capture capture, boolean friends) throws Exception {
        AccountNamePolicy.validate(baseName, actualName);
        if (capture == null || capture.code().isEmpty()) throw new IOException("新增账号必须提供新的 Code；仅有好友 GID 时请选择已有账号");
        if (friends && capture.gids().isEmpty()) throw new IOException("没有待同步的好友 GID");
        List<Account> existing = accounts();
        for (Account account : existing) {
            if (account.name.trim().equals(baseName.trim()) || account.name.trim().equals(actualName))
                throw new IOException("服务器已有同名备注；上游同名会更新旧账号，请改用不同备注或选择更新已有账号");
            if (capture.platform().id.equals(account.platform) && !capture.ownGid().isEmpty() && capture.ownGid().equals(knownGid(account)))
                throw new IOException("这个 GID 已存在于服务器，请选择对应账号更新，不要重复新增");
        }
        return existing;
    }
    public Account create(String baseName, String actualName, AppState.Capture capture, boolean friends) throws Exception {
        beginPush();
        try { return createPending(baseName, actualName, capture, friends); }
        finally { endPush(); }
    }
    private Account createPending(String baseName, String actualName, AppState.Capture capture, boolean friends) throws Exception {
        requirePending(capture, true, friends);
        List<Account> before = checkNewAccount(baseName, actualName, capture, friends);
        requirePending(capture, true, friends);
        JSONObject body = new JSONObject().put("name", actualName).put("code", capture.code())
                .put("platform", capture.platform().id).put("loginType", "manual");
        JSONObject response;
        try { response = call("POST", "/api/accounts", body, null); }
        catch (AuthException rejected) { throw rejected; }
        catch (Exception uncertain) {
            AppState.consumeCode(capture);
            throw new IOException("未能确认新增结果；为避免重复新增，Code 已清除。请先检查服务器账号列表，再重新获取");
        }
        AppState.consumeCode(capture);
        Account created = null;
        try {
            java.util.Set<String> previousIds = new java.util.HashSet<>();
            for (Account account : before) previousIds.add(account.id);
            for (Account account : parseAccounts(response)) {
                if (!previousIds.contains(account.id) && actualName.equals(account.name) && account.platform.equals(capture.platform().id)) {
                    if (created != null) throw new IOException("返回多个新增账号");
                    created = account;
                }
            }
            if (created == null) throw new IOException("未找到新增账号 ID");
        } catch (Exception invalidResponse) {
            throw new IOException("新增请求已提交，但无法确认服务器分配的账号 ID；Code 已清除，请到账号页刷新选择，不要重复新增");
        }
        if (!isCurrent()) throw new IOException("新增请求已提交，但服务器配置已变化；不会绑定到新配置，请在原服务器确认");
        settings.target(created);
        settings.rememberIdentity(connectionId, created, capture);
        AppState.log("推送", "新账号已创建并绑定服务器返回的 ID；由服务器按其规则自动启动");
        if (friends) {
            try { appendFriends(created.id, capture); }
            catch (Exception partial) { throw new IOException("新账号已创建并绑定，但好友同步未完成；可点击「只同步好友」重试，不要重复新增"); }
        }
        return created;
    }
    public String ping() throws Exception {
        JSONObject data = call("GET", "/api/ping", null, null).optJSONObject("data");
        return data == null ? "连接成功" : "连接成功 · 服务端 " + data.optString("version", "未知版本");
    }
    public boolean connected(Account account) throws Exception {
        if (!account.running) return false;
        JSONObject data = call("GET", "/api/status", null, account.id).optJSONObject("data");
        if (data == null || data.optJSONObject("connection") == null) throw new IOException("未返回账号连接状态");
        return data.getJSONObject("connection").optBoolean("connected");
    }
    public void push(String targetId, AppState.Capture capture, boolean code, boolean friends) throws Exception {
        if (!code && !friends) throw new IOException("没有待推送的数据");
        beginPush();
        try { pushPending(targetId, capture, code, friends); }
        finally { endPush(); }
    }
    private void pushPending(String targetId, AppState.Capture capture, boolean code, boolean friends) throws Exception {
        requirePending(capture, code, friends);
        if (targetId.isEmpty()) throw new IOException("请先选择目标账号");
        Account target = null;
        for (Account account : accounts()) if (account.id.equals(targetId)) target = account;
        if (target == null) throw new IOException("目标账号已不存在，请重新选择");
        if (!target.platform.equals(capture.platform().id)) throw new IOException("获取结果来自 " + capture.platform().label + "，请选择同一平台的账号");
        if (!capture.ownGid().isEmpty() && !target.gid.isEmpty() && !target.gid.equals(capture.ownGid()))
            throw new IOException("捕获身份与服务器 GID 不一致，已阻止更新");
        requirePending(capture, code, friends);
        if (code) {
            if (capture.code().isEmpty()) throw new IOException("没有可推送的 Code");
            JSONObject body = new JSONObject().put("id", target.id).put("name", target.name)
                    .put("code", capture.code()).put("platform", capture.platform().id).put("loginType", "manual");
            call("POST", "/api/accounts", body, null);
            AppState.consumeCode(capture);
            settings.rememberIdentity(connectionId, target, capture);
            AppState.log("推送", "Code 已更新；正在启动对应远程账号");
            try { call("POST", "/api/accounts/" + safeId(target.id) + "/start", new JSONObject(), null); }
            catch (Exception e) { throw new IOException("Code 已更新，但启动失败；请在管理页面检查，不要重复使用旧 Code"); }
        }
        if (friends) {
            appendFriends(target.id, capture);
        }
    }
    private void appendFriends(String targetId, AppState.Capture capture) throws Exception {
        requirePending(capture, false, true);
        List<String> gids = UrlPolicy.gids(String.join(",", capture.gids()), capture.ownGid());
        if (gids.isEmpty()) throw new IOException("没有好友 GID 可同步；Code 若已成功提交则不会再次使用");
        call("POST", "/api/friend-known-gids/batch-add", new JSONObject().put("gids", new JSONArray(gids)), targetId);
        AppState.consumeFriends(capture);
        AppState.log("推送", "好友已增量同步，共 " + gids.size() + " 个；未覆盖原有好友");
    }
    public JSONObject call(String method, String path, JSONObject body, String accountId) throws Exception {
        return call(method, path, body, accountId, 15_000);
    }
    private JSONObject call(String method, String path, JSONObject body, String accountId, int readTimeout) throws Exception {
        return new JSONObject(new String(request(method, path, body, accountId, readTimeout, false), StandardCharsets.UTF_8));
    }
    private byte[] request(String method, String path, JSONObject body, String accountId, int readTimeout, boolean image) throws Exception {
        if (!isCurrent()) throw new IOException("服务器配置已变更，请重新选择账号后重试");
        HttpURLConnection connection = (HttpURLConnection) URI.create(server + path).toURL().openConnection(Proxy.NO_PROXY);
        try {
            connection.setConnectTimeout(10_000); connection.setReadTimeout(readTimeout);
            connection.setInstanceFollowRedirects(false);
            connection.setRequestMethod(method);
            connection.setRequestProperty("x-admin-token", token);
            if (accountId != null) connection.setRequestProperty("x-account-id", safeId(accountId));
            connection.setRequestProperty("Accept", image ? "image/*" : "application/json");
            if (body != null) {
                connection.setDoOutput(true);
                connection.setRequestProperty("Content-Type", "application/json; charset=utf-8");
                byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
                connection.setFixedLengthStreamingMode(bytes.length);
                try (var output = connection.getOutputStream()) { output.write(bytes); }
            }
            int status = connection.getResponseCode();
            if (status == 401 || status == 403) throw new AuthException();
            if (status == 404 && path.startsWith("/api/mobile/offline-events?")) throw new RelayUnsupportedException();
            if (status >= 300 && status < 400) throw new IOException("服务器返回跳转；为保护 Token，请直接填写最终服务器地址");
            if (status == 404 && path.startsWith("/api/wx-login/")) throw new IOException("扫码任务已失效，或服务器尚未支持微信扫码登录，请重新获取或更新服务器");
            if (status == 408 || status == 429 || status == 502 || status == 503 || status == 504) throw new TemporaryNetworkException();
            if (status != 200) throw new IOException("服务器请求失败（HTTP " + status + "）");
            byte[] bytes;
            try (InputStream input = connection.getInputStream()) { bytes = readBounded(input, 4 * 1024 * 1024); }
            if (!isCurrent()) throw new IOException("服务器配置已变更，请重新开始");
            if (image) return bytes;
            JSONObject result = new JSONObject(new String(bytes, StandardCharsets.UTF_8));
            if (!result.optBoolean("ok")) throw new IOException("服务器拒绝操作，请到管理页面检查账号和权限");
            return bytes;
        } catch (javax.net.ssl.SSLException e) {
            throw new IOException("服务器 HTTPS 证书验证失败；不会忽略 TLS 错误");
        } catch (java.net.UnknownHostException | java.net.SocketException | java.net.SocketTimeoutException e) {
            throw new TemporaryNetworkException();
        } finally { connection.disconnect(); }
    }
    public record WxTask(String id, long expiresAt) {}
    public record WxDraft(String id, String baseName, String name, java.util.Set<String> previousIds) {}
    public static final class UncertainWxSaveException extends IOException {
        UncertainWxSaveException() { super("登录提交结果未能确认。请先到服务器账号页检查，勿重复新增；本次不会自动重试提交。"); }
    }
    public WxDraft prepareWxAccount(String targetId, String baseName, String actualName) throws Exception {
        List<Account> list = accounts();
        if (!targetId.isEmpty()) {
            for (Account account : list) if (account.id.equals(targetId)) {
                if (!account.platform.equals("wx")) throw new IOException("只能更新微信账号");
                return new WxDraft(account.id, "", account.name, java.util.Set.of());
            }
            throw new IOException("目标账号已不存在，请重新选择");
        }
        AccountNamePolicy.validate(baseName, actualName);
        java.util.Set<String> ids = new java.util.HashSet<>();
        for (Account account : list) {
            if (account.name.trim().equals(baseName.trim()) || account.name.trim().equals(actualName))
                throw new IOException("已有同名账号，请选择更新已有账号或更换备注");
            ids.add(account.id);
        }
        return new WxDraft("", baseName, actualName, ids);
    }
    public WxTask createWxTask() throws Exception {
        JSONObject data = call("POST", "/api/wx-login/tasks", new JSONObject().put("app_id", "wx5306c5978fdb76e4"), null).getJSONObject("data");
        String id = safeId(data.getString("task_id"));
        long expiresAt = data.getLong("expires_at") * 1000L;
        return new WxTask(id, expiresAt);
    }
    private String wxPath(WxTask task) { return "/api/wx-login/tasks/" + safeId(task.id()); }
    public byte[] wxQr(WxTask task) throws Exception {
        // Construct the path ourselves: a server-provided qr_url must never receive the admin token off-origin.
        return request("GET", wxPath(task) + "/qr", null, null, 15_000, true);
    }
    public String wxStatus(WxTask task) throws Exception {
        // The server holds this request while waiting for WeChat; ordinary API reads use 15 seconds.
        return call("GET", wxPath(task) + "/status", null, null, 40_000).getJSONObject("data").getString("status");
    }
    public void cancelWxTask(WxTask task) throws Exception { call("DELETE", wxPath(task), null, null); }
    public Account finishWxLogin(WxTask task, WxDraft draft) throws Exception {
        WxDraft checked = prepareWxAccount(draft.id(), draft.baseName(), draft.name());
        if (!checked.name().equals(draft.name())) throw new IOException("目标账号备注已变化，请重新确认");
        call("POST", wxPath(task) + "/confirm", new JSONObject(), null, 40_000);
        String code = call("POST", wxPath(task) + "/code", new JSONObject(), null, 40_000).getJSONObject("data").getString("code");
        if (code.isBlank()) throw new IOException("服务器未取得农场 Code，请重新扫码");
        JSONObject body = new JSONObject().put("name", draft.name()).put("code", code).put("platform", "wx")
                .put("loginType", "wechat_qr").put("wxLoginTaskId", task.id());
        if (!draft.id().isEmpty()) body.put("id", draft.id());
        JSONObject response;
        try { response = call("POST", "/api/accounts", body, null); }
        catch (AuthException rejected) { throw rejected; }
        catch (Exception uncertain) { throw new UncertainWxSaveException(); }
        Account saved = null;
        try {
            for (Account account : parseAccounts(response)) {
                boolean identity = draft.id().isEmpty() ? !checked.previousIds().contains(account.id) : account.id.equals(draft.id());
                if (identity && account.platform.equals("wx") && account.name.equals(draft.name())) {
                    if (saved != null) throw new UncertainWxSaveException();
                    saved = account;
                }
            }
            if (saved == null || !isCurrent()) throw new UncertainWxSaveException();
        } catch (Exception invalid) { throw new UncertainWxSaveException(); }
        settings.target(saved);
        AppState.log("微信扫码", "农场 Code 与扫码会话已提交服务器；正在检查登录与好友");
        return saved;
    }
    public int wechatFriendCount(String targetId) throws Exception {
        JSONArray data = call("GET", "/api/friends?forceSync=true", null, targetId, 40_000).getJSONArray("data");
        java.util.Set<String> gids = new java.util.HashSet<>();
        for (int i = 0; i < data.length(); i++) {
            String gid = data.getJSONObject(i).optString("gid");
            if (!gid.matches("[1-9][0-9]*")) throw new IOException("好友响应缺少有效 GID，请在服务器确认");
            gids.add(gid);
        }
        return gids.size();
    }
    public record OfflineEvent(String eventId, String accountId, String accountUuid, String platform) {}
    public record OfflineBatch(String cursor, List<OfflineEvent> events) {}
    public OfflineBatch offlineEvents(String cursor, java.util.Set<String> selected) throws Exception {
        if (!cursor.matches("[A-Za-z0-9_:-]{0,100}")) throw new IOException("通知读取位置无效");
        List<String> ids = new ArrayList<>();
        for (String id : selected) ids.add(safeId(id));
        if (ids.isEmpty()) throw new IOException("请先选择需要提醒的账号");
        String path = "/api/mobile/offline-events?after=" + cursor + "&wait=25&accounts=" + String.join(",", ids);
        JSONObject data = call("GET", path, null, null, 35_000).getJSONObject("data");
        String next = data.getString("cursor");
        if (!next.matches("[A-Za-z0-9_-]{1,64}:[0-9]{1,20}")) throw new IOException("服务器通知读取位置无效");
        JSONArray array = data.getJSONArray("events"); List<OfflineEvent> events = new ArrayList<>();
        for (int i = 0; i < array.length(); i++) {
            JSONObject event = array.getJSONObject(i);
            String id = safeId(event.getString("accountId")), uuid = event.getString("accountUuid");
            if (!event.getString("type").equals("relogin_required") || uuid.isBlank()) throw new IOException("服务器通知格式不正确");
            if (selected.contains(id)) events.add(new OfflineEvent(event.getString("eventId"), id, uuid, event.getString("platform")));
        }
        return new OfflineBatch(next, events);
    }
    public static byte[] readBounded(InputStream input, int limit) throws IOException {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        byte[] buffer = new byte[8192]; int n;
        while ((n = input.read(buffer)) != -1) {
            if (output.size() + n > limit) throw new IOException("响应过大，已停止读取");
            output.write(buffer, 0, n);
        }
        return output.toByteArray();
    }
    private static String safeId(String value) {
        if (!value.matches("[A-Za-z0-9_-]{1,128}")) throw new IllegalArgumentException("账号 ID 格式不正确");
        return value;
    }
}
