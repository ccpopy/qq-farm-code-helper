package io.github.ccpopy.farmhelper;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.io.IOException;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.Arrays;

public final class UpdateManager {
    private static final String API = "https://api.github.com/repos/" + UrlPolicy.REPOSITORY + "/releases?per_page=30";
    private static final long MAX_APK = 64L * 1024 * 1024;
    public record Release(String version, String notes, String url, String asset, long size, String sha256) {}
    private UpdateManager() {}
    public static Release check(SettingsStore settings) throws Exception {
        byte[] bytes;
        try { bytes = fetch(API, 2 * 1024 * 1024); }
        catch (IOException e) {
            if (!settings.accelerated()) throw e;
            AppState.log("更新", "直连检查失败，尝试已配置的加速源");
            bytes = fetch(mirror(settings, API), 2 * 1024 * 1024);
        }
        JSONArray releases = new JSONArray(new String(bytes, StandardCharsets.UTF_8));
        Release best = null;
        for (int i = 0; i < releases.length(); i++) {
            JSONObject release = releases.getJSONObject(i);
            String tag = release.optString("tag_name");
            if (!tag.matches("android-v[0-9]+\\.[0-9]+\\.[0-9]+") || release.optBoolean("prerelease") || release.optBoolean("draft")) continue;
            String version = tag.substring("android-v".length());
            JSONArray assets = release.optJSONArray("assets"); if (assets == null) continue;
            for (int n = 0; n < assets.length(); n++) {
                JSONObject asset = assets.getJSONObject(n);
                if (!asset.optString("name").equals("QQFarmCodeHelper-" + tag + ".apk")) continue;
                String digest = asset.optString("digest");
                if (!digest.matches("sha256:[0-9a-fA-F]{64}")) continue;
                String url = asset.optString("browser_download_url");
                if (!url.equals("https://github.com/" + UrlPolicy.REPOSITORY + "/releases/download/" + tag + "/" + asset.optString("name"))) continue;
                long size = asset.optLong("size");
                if (size <= 0 || size > MAX_APK) continue;
                if (best == null || compare(version, best.version) > 0)
                    best = new Release(version, release.optString("body"), "https://github.com/" + UrlPolicy.REPOSITORY + "/releases/tag/" + tag, url, size, digest.substring(7));
            }
        }
        return best;
    }
    public static File download(Context context, SettingsStore settings, Release release) throws Exception {
        String source = settings.accelerated() ? mirror(settings, release.asset) : release.asset;
        byte[] bytes = fetch(source, (int)Math.min(release.size + 1, MAX_APK));
        if (bytes.length != release.size) throw new IOException("更新包大小不一致，已拒绝安装");
        StringBuilder hex = new StringBuilder();
        for (byte b : MessageDigest.getInstance("SHA-256").digest(bytes)) hex.append(String.format(java.util.Locale.ROOT, "%02x", b));
        if (!hex.toString().equalsIgnoreCase(release.sha256)) throw new IOException("更新包 SHA-256 不匹配，已拒绝安装");
        File directory = new File(context.getCacheDir(), "updates");
        if (!directory.isDirectory() && !directory.mkdirs()) throw new IOException("无法创建更新缓存");
        File file = new File(directory, "verified-update.apk");
        Files.write(file.toPath(), bytes);
        try {
            PackageManager pm = context.getPackageManager();
            PackageInfo installed = pm.getPackageInfo(context.getPackageName(), PackageManager.GET_SIGNING_CERTIFICATES);
            PackageInfo downloaded = pm.getPackageArchiveInfo(file.getAbsolutePath(), PackageManager.GET_SIGNING_CERTIFICATES);
            if (downloaded == null || !context.getPackageName().equals(downloaded.packageName)
                    || !release.version.equals(downloaded.versionName) || downloaded.getLongVersionCode() <= installed.getLongVersionCode()
                    || downloaded.signingInfo == null || installed.signingInfo == null
                    || !sameSigners(installed.signingInfo.getApkContentsSigners(), downloaded.signingInfo.getApkContentsSigners()))
                throw new IOException("更新包身份、版本或签名不匹配；调试版不能直接升级为正式签名版");
            return file;
        } catch (Exception e) { Files.deleteIfExists(file.toPath()); throw e; }
    }
    private static boolean sameSigners(Signature[] a, Signature[] b) {
        if (a.length != b.length || a.length == 0) return false;
        for (Signature signature : a) if (!Arrays.asList(b).contains(signature)) return false;
        return true;
    }
    public static int compare(String a, String b) {
        String[] left = a.split("\\."), right = b.split("\\.");
        if (left.length != 3 || right.length != 3) throw new IllegalArgumentException("版本号格式不正确");
        for (int i = 0; i < 3; i++) {
            int result = Long.compare(Long.parseLong(left[i]), Long.parseLong(right[i]));
            if (result != 0) return result;
        }
        return 0;
    }
    private static String mirror(SettingsStore settings, String url) {
        return UrlPolicy.server(settings.mirror(), false).toString() + "/" + url;
    }
    private static byte[] fetch(String url, int limit) throws Exception {
        HttpURLConnection connection = (HttpURLConnection) URI.create(url).toURL().openConnection(Proxy.NO_PROXY);
        try {
            connection.setConnectTimeout(10_000); connection.setReadTimeout(30_000);
            connection.setRequestProperty("User-Agent", "FarmHelperAndroid/" + BuildConfig.VERSION_NAME);
            connection.setRequestProperty("Accept", "application/json,application/octet-stream");
            connection.setInstanceFollowRedirects(false);
            int status = connection.getResponseCode();
            // GitHub release binaries redirect to an HTTPS object-storage URL. No credentials are sent here.
            if (status >= 300 && status < 400) {
                String location = connection.getHeaderField("Location");
                URI target = location == null ? null : URI.create(location);
                if (target == null || !"https".equalsIgnoreCase(target.getScheme()) || target.getHost() == null || target.getUserInfo() != null)
                    throw new IOException("更新源跳转无效");
                return fetchRedirect(target.toString(), limit);
            }
            if (status != 200) throw new IOException("更新源请求失败（HTTP " + status + "）");
            try (var input = connection.getInputStream()) { return ApiClient.readBounded(input, limit); }
        } finally { connection.disconnect(); }
    }
    private static byte[] fetchRedirect(String url, int limit) throws Exception {
        HttpURLConnection connection = (HttpURLConnection) URI.create(url).toURL().openConnection(Proxy.NO_PROXY);
        try {
            connection.setInstanceFollowRedirects(false); connection.setConnectTimeout(10_000); connection.setReadTimeout(30_000);
            if (connection.getResponseCode() != 200) throw new IOException("更新包下载失败");
            try (var input = connection.getInputStream()) { return ApiClient.readBounded(input, limit); }
        } finally { connection.disconnect(); }
    }
}
