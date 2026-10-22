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

public final class SettingsStore {
    private static final String KEY = "farmhelper.settings.aes.v1";
    private final SharedPreferences prefs;
    public SettingsStore(Context context) { this(context, "settings"); }
    SettingsStore(Context context, String name) { prefs = context.getSharedPreferences(name, Context.MODE_PRIVATE); }
    public String server() { return prefs.getString("server", ""); }
    public String connectionId() { return prefs.getString("connectionId", ""); }
    public boolean hasToken() { return !prefs.getString("tokenCipher", "").isEmpty(); }
    public boolean allowHttp() { return prefs.getBoolean("allowHttp", false); }
    public boolean friendCapture() { return prefs.getBoolean("friendCapture", true); }
    public boolean passthrough() { return prefs.getBoolean("passthrough", false); }
    public void passthrough(boolean enabled) { prefs.edit().putBoolean("passthrough", enabled).apply(); }
    public boolean accelerated() { return prefs.getBoolean("accelerated", true); }
    public String mirror() { return prefs.getString("mirror", "https://ghfast.top"); }
    public String targetId() { return prefs.getString("targetId", ""); }
    public String targetName() { return prefs.getString("targetName", "未选择账号"); }
    public String targetGid() { return prefs.getString("targetGid", ""); }
    public int pollSeconds() { return Math.max(30, Math.min(900, prefs.getInt("pollSeconds", 60))); }
    public Set<String> monitoredIds() { return new HashSet<>(prefs.getStringSet("monitoredIds", java.util.Collections.emptySet())); }
    public void monitor(Set<String> ids) { prefs.edit().putStringSet("monitoredIds", new HashSet<>(ids)).apply(); }
    public void target(ApiClient.Account account) {
        prefs.edit().putString("targetId", account.id).putString("targetName", account.name).putString("targetGid", account.gid).apply();
    }
    public void options(boolean friends, boolean accelerated, String mirror, int seconds) {
        prefs.edit().putBoolean("friendCapture", friends).putBoolean("accelerated", accelerated)
                .putString("mirror", mirror).putInt("pollSeconds", Math.max(30, Math.min(900, seconds))).apply();
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
                .remove("targetId").remove("targetName").remove("targetGid").remove("monitoredIds");
        if (!edit.commit()) throw new IllegalStateException("保存设置失败");
        return changed;
    }
    public synchronized void forgetServer() {
        prefs.edit().remove("server").remove("tokenCipher").remove("allowHttp").remove("connectionId")
                .remove("targetId").remove("targetName").remove("targetGid").remove("monitoredIds").apply();
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
