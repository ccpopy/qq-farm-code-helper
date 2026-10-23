package io.github.ccpopy.farmhelper;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.util.HashSet;
import java.util.Set;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import org.json.JSONObject;

public final class SettingsStore {
    private static final String KEY = "farmhelper.settings.aes.v1";
    private final SharedPreferences prefs;
    public SettingsStore(Context context) { this(context, "settings"); }
    SettingsStore(Context context, String name) { prefs = context.getSharedPreferences(name, Context.MODE_PRIVATE); }
    public String server() { return prefs.getString("server", ""); }
    public String connectionId() { return prefs.getString("connectionId", ""); }
    public boolean hasToken() { return !prefs.getString("tokenCipher", "").isEmpty(); }
    public boolean allowHttp() { return prefs.getBoolean("allowHttp", false); }
    public boolean friendCapture() { return prefs.getBoolean(friendCaptureKey(), capturePlatform() == CapturePlatform.QQ); }
    private String friendCaptureKey() { return capturePlatform() == CapturePlatform.WECHAT ? "wechatFriendCapture" : "friendCapture"; }
    public CapturePlatform capturePlatform() {
        try { return CapturePlatform.fromId(prefs.getString("capturePlatform", "qq")); }
        catch (IllegalArgumentException unsupported) { return CapturePlatform.QQ; }
    }
    public void capturePlatform(CapturePlatform platform) {
        if (platform == capturePlatform()) return;
        prefs.edit().putString("capturePlatform", platform.id)
                .remove("targetId").remove("targetName").remove("targetGid").remove("targetPlatform").apply();
    }
    public boolean passthrough() { return prefs.getBoolean("passthrough", false); }
    public void passthrough(boolean enabled) { prefs.edit().putBoolean("passthrough", enabled).apply(); }
    public void friendCapture(boolean enabled) { prefs.edit().putBoolean(friendCaptureKey(), enabled).apply(); }
    /** Whether a finished capture opens the account-matching flow by itself. Pushing still needs confirmation. */
    public boolean autoPush() { return prefs.getBoolean("autoPush", true); }
    public void autoPush(boolean enabled) { prefs.edit().putBoolean("autoPush", enabled).apply(); }
    public boolean pushCode() { return prefs.getBoolean("pushCode", true); }
    public void pushCode(boolean enabled) { prefs.edit().putBoolean("pushCode", enabled).apply(); }
    public boolean pushFriends() { return prefs.getBoolean("pushFriends", true); }
    public void pushFriends(boolean enabled) { prefs.edit().putBoolean("pushFriends", enabled).apply(); }
    public void accelerated(boolean enabled) { prefs.edit().putBoolean("accelerated", enabled).apply(); }
    public void mirror(String base) { prefs.edit().putString("mirror", base).apply(); }
    public void pollSeconds(int seconds) { prefs.edit().putInt("pollSeconds", Math.max(1, seconds)).apply(); }
    /** The user's on/off choice for offline alerts; the service only runs while at least one account is selected. */
    public boolean monitorEnabled() { return prefs.getBoolean("monitorEnabled", false); }
    public void monitorEnabled(boolean enabled) { prefs.edit().putBoolean("monitorEnabled", enabled).apply(); }
    /** Screenshots and screen recording are blocked unless the user explicitly allows them. */
    public boolean allowScreenshots() { return prefs.getBoolean("allowScreenshots", false); }
    public void allowScreenshots(boolean allowed) { prefs.edit().putBoolean("allowScreenshots", allowed).apply(); }
    public boolean overlayEnabled() { return prefs.getBoolean("overlayEnabled", false); }
    public void overlayEnabled(boolean enabled) { prefs.edit().putBoolean("overlayEnabled", enabled).apply(); }
    public boolean accelerated() { return prefs.getBoolean("accelerated", true); }
    public String mirror() { return prefs.getString("mirror", "https://ghfast.top"); }
    public String targetId() { return prefs.getString("targetId", ""); }
    public String targetName() { return prefs.getString("targetName", "未选择账号"); }
    public String targetGid() { return prefs.getString("targetGid", ""); }
    public String targetPlatform() { return prefs.getString("targetPlatform", "qq"); }
    public int pollSeconds() { return Math.max(1, prefs.getInt("pollSeconds", 60)); }
    public Set<String> monitoredIds() { return new HashSet<>(prefs.getStringSet("monitoredIds", java.util.Collections.emptySet())); }
    public void monitor(Set<String> ids) { prefs.edit().putStringSet("monitoredIds", new HashSet<>(ids)).apply(); }
    public void target(ApiClient.Account account) {
        prefs.edit().putString("targetId", account.id).putString("targetName", account.name).putString("targetGid", account.gid)
                .putString("targetPlatform", account.platform).apply();
    }
    /** A user-confirmed destination is local to this server and this account incarnation. */
    public synchronized String rememberedGid(String connection, ApiClient.Account account) {
        if (!connection.equals(connectionId()) || account.uuid.isEmpty()) return "";
        try {
            JSONObject entry = new JSONObject(prefs.getString("accountIdentities", "{}")).optJSONObject(account.id);
            if (entry == null || !account.uuid.equals(entry.optString("uuid"))
                    || !account.platform.equals(entry.optString("platform")) || !account.qq.equals(entry.optString("qq"))) return "";
            return AccountIdentity.gid(entry.optString("gid"));
        } catch (Exception invalid) { return ""; }
    }
    public synchronized void rememberIdentity(String connection, ApiClient.Account account, AppState.Capture capture) {
        if (!connection.equals(connectionId()) || !account.platform.equals(capture.platform().id)) return;
        try {
            JSONObject entries;
            try { entries = new JSONObject(prefs.getString("accountIdentities", "{}")); }
            catch (Exception invalid) { entries = new JSONObject(); }
            // A Code-only replacement must invalidate a previous association, not silently reuse it.
            entries.remove(account.id);
            String gid = AccountIdentity.gid(capture.ownGid());
            if (!account.uuid.isEmpty() && !gid.isEmpty()) entries.put(account.id, new JSONObject()
                    .put("uuid", account.uuid).put("platform", account.platform).put("qq", account.qq).put("gid", gid));
            prefs.edit().putString("accountIdentities", entries.toString()).apply();
        } catch (Exception invalid) { /* Identity suggestions must not turn an accepted push into a failed one. */ }
    }
    public void options(boolean friends, boolean accelerated, String mirror, int seconds) {
        prefs.edit().putBoolean(friendCaptureKey(), friends).putBoolean("accelerated", accelerated)
                .putString("mirror", mirror).putInt("pollSeconds", Math.max(1, seconds)).apply();
    }
    public synchronized String token() throws Exception {
        String encoded = prefs.getString("tokenCipher", "");
        if (encoded.isEmpty()) return "";
        byte[] data = Base64.decode(encoded, Base64.NO_WRAP);
        if (data.length < 29) throw new IllegalStateException("凭据存储损坏，请重新填写 Token");
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(128, data, 0, 12));
        return new String(cipher.doFinal(data, 12, data.length - 12), StandardCharsets.UTF_8);
    }
    public synchronized boolean saveServer(String address, String token, boolean http) throws Exception {
        String base = UrlPolicy.server(address, http).toString();
        String secret = token.trim();
        if (secret.isEmpty()) {
            if (!base.equals(server())) throw new IllegalArgumentException("首次配置或更换服务器地址时，请重新填写管理 Token");
            secret = token();
        }
        if (secret.isEmpty() || secret.length() > 8192 || secret.chars().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("请填写有效的管理 Token");
        String previous;
        try { previous = token(); } catch (Exception damagedCredential) { previous = null; }
        boolean changed = !base.equals(server()) || !secret.equals(previous) || connectionId().isEmpty();
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, key());
        byte[] ciphertext = cipher.doFinal(secret.getBytes(StandardCharsets.UTF_8));
        byte[] data = new byte[12 + ciphertext.length];
        System.arraycopy(cipher.getIV(), 0, data, 0, 12);
        System.arraycopy(ciphertext, 0, data, 12, ciphertext.length);
        SharedPreferences.Editor edit = prefs.edit().putString("server", base).putBoolean("allowHttp", http)
                .putString("tokenCipher", Base64.encodeToString(data, Base64.NO_WRAP));
        if (changed) edit.putString("connectionId", java.util.UUID.randomUUID().toString())
                .remove("targetId").remove("targetName").remove("targetGid").remove("targetPlatform").remove("monitoredIds").remove("accountIdentities");
        if (!edit.commit()) throw new IllegalStateException("保存设置失败");
        return changed;
    }
    public synchronized void forgetServer() {
        prefs.edit().remove("server").remove("tokenCipher").remove("allowHttp").remove("connectionId")
                .remove("targetId").remove("targetName").remove("targetGid").remove("targetPlatform").remove("monitoredIds").remove("accountIdentities").apply();
    }
    private SecretKey key() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (!store.containsAlias(KEY)) {
            KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
            generator.init(new KeyGenParameterSpec.Builder(KEY, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build());
            generator.generateKey();
        }
        return (SecretKey) store.getKey(KEY, null);
    }
}
