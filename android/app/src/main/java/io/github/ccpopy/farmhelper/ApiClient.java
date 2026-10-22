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
    private final String server;
    private final String token;
    private final SettingsStore settings;
    public final String connectionId;
    public static final class AuthException extends IOException {
        public AuthException() { super("管理 Token 已失效或权限不足，请重新登录服务器并复制 Token"); }
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
    public static final class Account {
        public final String id, name, platform, gid;
        public final boolean running;
        Account(JSONObject json) {
            id = json.optString("id"); name = json.optString("name", id);
            platform = json.optString("platform", "qq"); gid = json.optString("gid");
            running = json.optBoolean("running");
        }
        public String label() { return name + "  ·  " + (running ? "运行中" : "已停止") + "  ·  ID " + id; }
    }
    public List<Account> accounts() throws Exception {
        return parseAccounts(call("GET", "/api/accounts", null, null));
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
            if (!capture.ownGid().isEmpty() && capture.ownGid().equals(account.gid))
                throw new IOException("这个 GID 已存在于服务器，请选择对应账号更新，不要重复新增");
        }
        return existing;
    }
    public Account create(String baseName, String actualName, AppState.Capture capture, boolean friends) throws Exception {
        List<Account> before = checkNewAccount(baseName, actualName, capture, friends);
        if (AppState.capture() != capture) throw new IOException("结果已过期或变更，请重新获取后确认");
        JSONObject body = new JSONObject().put("name", actualName).put("code", capture.code())
                .put("platform", "qq").put("loginType", "manual");
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
                if (!previousIds.contains(account.id) && actualName.equals(account.name) && account.platform.equals("qq")) {
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
        if (targetId.isEmpty()) throw new IOException("请先选择目标账号");
        Account target = null;
        for (Account account : accounts()) if (account.id.equals(targetId)) target = account;
        if (target == null) throw new IOException("目标账号已不存在，请重新选择");
        if (!target.platform.equals("qq")) throw new IOException("当前抓取仅支持 QQ，请选择 QQ 账号");
        if (!capture.ownGid().isEmpty() && !target.gid.isEmpty() && !target.gid.equals(capture.ownGid()))
            throw new IOException("捕获身份与服务器 GID 不一致，已阻止更新");
        if (code) {
            if (capture.code().isEmpty()) throw new IOException("没有可推送的 Code");
            JSONObject body = new JSONObject().put("id", target.id).put("name", target.name)
                    .put("code", capture.code()).put("platform", "qq").put("loginType", "manual");
            call("POST", "/api/accounts", body, null);
            AppState.consumeCode(capture);
            AppState.log("推送", "Code 已更新；正在启动对应远程账号");
            try { call("POST", "/api/accounts/" + safeId(target.id) + "/start", new JSONObject(), null); }
            catch (Exception e) { throw new IOException("Code 已更新，但启动失败；请在管理页面检查，不要重复使用旧 Code"); }
        }
        if (friends) {
            appendFriends(target.id, capture);
        }
    }
    private void appendFriends(String targetId, AppState.Capture capture) throws Exception {
        List<String> gids = UrlPolicy.gids(String.join(",", capture.gids()), capture.ownGid());
        if (gids.isEmpty()) throw new IOException("没有好友 GID 可同步；Code 若已成功提交则不会再次使用");
        call("POST", "/api/friend-known-gids/batch-add", new JSONObject().put("gids", new JSONArray(gids)), targetId);
        AppState.consumeFriends(capture);
        AppState.log("推送", "好友已增量同步，共 " + gids.size() + " 个；未覆盖原有好友");
    }
    public JSONObject call(String method, String path, JSONObject body, String accountId) throws Exception {
        if (!isCurrent()) throw new IOException("服务器配置已变更，请重新选择账号后重试");
        HttpURLConnection connection = (HttpURLConnection) URI.create(server + path).toURL().openConnection(Proxy.NO_PROXY);
        try {
            connection.setConnectTimeout(10_000); connection.setReadTimeout(15_000);
            connection.setInstanceFollowRedirects(false);
            connection.setRequestMethod(method);
            connection.setRequestProperty("x-admin-token", token);
            if (accountId != null) connection.setRequestProperty("x-account-id", safeId(accountId));
            connection.setRequestProperty("Accept", "application/json");
            if (body != null) {
                connection.setDoOutput(true);
                connection.setRequestProperty("Content-Type", "application/json; charset=utf-8");
                byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
                connection.setFixedLengthStreamingMode(bytes.length);
                try (var output = connection.getOutputStream()) { output.write(bytes); }
            }
            int status = connection.getResponseCode();
            if (status == 401 || status == 403) throw new AuthException();
            if (status >= 300 && status < 400) throw new IOException("服务器返回跳转；为保护 Token，请直接填写最终服务器地址");
            if (status != 200) throw new IOException("服务器请求失败（HTTP " + status + "）");
            JSONObject result;
            try (InputStream input = connection.getInputStream()) { result = new JSONObject(new String(readBounded(input, 4 * 1024 * 1024), StandardCharsets.UTF_8)); }
            if (!result.optBoolean("ok")) throw new IOException("服务器拒绝操作，请到管理页面检查账号和权限");
            return result;
        } catch (javax.net.ssl.SSLException e) {
            throw new IOException("服务器 HTTPS 证书验证失败；不会忽略 TLS 错误");
        } catch (java.net.SocketTimeoutException e) {
            throw new IOException("连接超时，请检查服务器地址与网络");
        } finally { connection.disconnect(); }
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
