package io.github.ccpopy.farmhelper;

import java.net.URI;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;

/** Pure Java validation shared by capture, manual input and server requests. */
public final class UrlPolicy {
    public static final String FARM_HOST = "gate-obt.nqf.qq.com";
    public static final String REPOSITORY = "ccpopy/qq-farm-code-helper";
    private UrlPolicy() {}

    public static URI server(String input, boolean allowHttp) {
        URI uri;
        try { uri = URI.create(input.trim()); }
        catch (RuntimeException e) { throw new IllegalArgumentException("服务器地址格式不正确"); }
        if ((!"https".equalsIgnoreCase(uri.getScheme()) && !(allowHttp && "http".equalsIgnoreCase(uri.getScheme())))
                || uri.getHost() == null || uri.getUserInfo() != null || uri.getRawQuery() != null || uri.getRawFragment() != null
                || uri.getPort() == 0 || uri.getPort() > 65535) {
            throw new IllegalArgumentException("请填写 HTTPS 地址，不要附带 Token、查询参数或用户名；HTTP 需在设置中单独允许");
        }
        return URI.create(uri.toString().replaceAll("/+$", ""));
    }

    public static String captureCode(String requestTarget, String host) {
        if (!FARM_HOST.equalsIgnoreCase(host)) throw new IllegalArgumentException("不是目标农场网关");
        URI uri;
        try { uri = URI.create(requestTarget); }
        catch (RuntimeException e) { throw new IllegalArgumentException("登录地址格式不正确"); }
        if (uri.isAbsolute() && (!"wss".equalsIgnoreCase(uri.getScheme()) && !"https".equalsIgnoreCase(uri.getScheme())
                || !FARM_HOST.equalsIgnoreCase(uri.getHost()) || uri.getUserInfo() != null
                || uri.getPort() != -1 && uri.getPort() != 443)) throw new IllegalArgumentException("登录地址来源不正确");
        if (!uri.isAbsolute() && uri.getRawAuthority() != null || !"/prod/ws".equals(uri.getRawPath()) || uri.getRawFragment() != null) throw new IllegalArgumentException("不是农场登录路径");
        String result = null;
        if (uri.getRawQuery() != null) for (String pair : uri.getRawQuery().split("&")) {
            String[] parts = pair.split("=", 2);
            if (decode(parts[0]).equals("code")) {
                if (result != null || parts.length != 2) throw new IllegalArgumentException("Code 参数重复或缺失");
                result = validateCode(decode(parts[1]));
            }
        }
        if (result == null) throw new IllegalArgumentException("未找到 Code");
        return result;
    }

    public static String manualCode(String input) {
        String value = input.trim();
        if (value.contains("://")) return captureCode(value, FARM_HOST);
        return validateCode(value);
    }

    private static String validateCode(String value) {
        if (value.isEmpty() || value.length() > 4096 || value.chars().anyMatch(c -> Character.isWhitespace(c) || Character.isISOControl(c)))
            throw new IllegalArgumentException("Code 为空、过长或包含空白字符");
        return value;
    }
    private static String decode(String value) {
        // URL query values are percent encoded; a literal '+' in a login Code
        // is not an HTML form space (same semantics as decodeURIComponent).
        try { return URLDecoder.decode(value.replace("+", "%2B"), "UTF-8"); }
        catch (Exception e) { throw new IllegalArgumentException("参数编码不正确"); }
    }
    public static List<String> gids(String input, String ownGid) {
        LinkedHashSet<String> result = new LinkedHashSet<>();
        for (String part : input.split("[,，;；\\s]+")) {
            if (part.isEmpty()) continue;
            if (!part.matches("[1-9][0-9]{0,15}")) throw new IllegalArgumentException("GID 必须是正整数");
            long n;
            try { n = Long.parseLong(part); }
            catch (NumberFormatException e) { throw new IllegalArgumentException("GID 超出范围"); }
            if (n > 9_007_199_254_740_991L) throw new IllegalArgumentException("GID 超出服务端安全整数范围");
            String normalized = Long.toString(n);
            if (!normalized.equals(ownGid)) result.add(normalized);
        }
        return new ArrayList<>(result);
    }
}
