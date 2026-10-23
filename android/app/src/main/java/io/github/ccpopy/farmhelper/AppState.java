package io.github.ccpopy.farmhelper;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import java.time.LocalTime;
import java.time.format.DateTimeFormatter;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CopyOnWriteArraySet;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Only bounded, redacted diagnostics and short-lived credentials live in memory. */
public final class AppState {
    public static final ExecutorService IO = Executors.newFixedThreadPool(3);
    public static SettingsStore settings;
    public static volatile boolean capturing;
    public static volatile boolean captureReady;
    public static volatile boolean monitoring;
    public static volatile boolean tlsVerified;
    public static volatile int gatewayConnections, gatewayTlsAttempts;
    public static volatile int socksAccepted, socksAuthenticated, socksRejected;
    public static volatile long tunnelTxPackets, tunnelRxPackets;
    public static volatile String proxyFailure = "none";
    public static volatile int tcpPassthroughs, udpAssociations, udpPacketsOut, udpPacketsIn, udpDropped;
    static final java.util.concurrent.atomic.AtomicLong tcpBytesOut = new java.util.concurrent.atomic.AtomicLong();
    static final java.util.concurrent.atomic.AtomicLong tcpBytesIn = new java.util.concurrent.atomic.AtomicLong();
    public static volatile boolean capturePassthrough;
    public static volatile CapturePlatform capturePlatform = CapturePlatform.QQ;
    public static volatile boolean vpnIpv6;
    private static final java.util.LinkedHashMap<String, Integer> tlsHosts = new java.util.LinkedHashMap<>();
    private static final java.util.HashSet<String> tlsReplies = new java.util.HashSet<>();
    static synchronized void resetConnections() {
        tlsHosts.clear(); tlsReplies.clear(); tcpPassthroughs = udpAssociations = udpPacketsOut = udpPacketsIn = udpDropped = 0;
        tcpBytesOut.set(0); tcpBytesIn.set(0);
    }
    static synchronized void observeTlsReply(String host) { if (tlsHosts.containsKey(host)) { tlsReplies.add(host); changed(); } }
    static synchronized boolean farmTrafficReplied() { return tlsReplies.contains(UrlPolicy.FARM_HOST); }
    static synchronized void observeTls(String host) {
        if (host == null || host.isEmpty() || host.length() > 253 || !host.matches("[a-zA-Z0-9._-]+")) return;
        host = host.toLowerCase(java.util.Locale.ROOT);
        if (!tlsHosts.containsKey(host) && tlsHosts.size() >= 40) return;
        tlsHosts.put(host, tlsHosts.getOrDefault(host, 0) + 1); changed();
    }
    public static synchronized String connectionText() {
        StringBuilder text = new StringBuilder("本次接管：").append(capturePlatform.label)
                .append("\nTCP 透传：").append(tcpPassthroughs).append(" · UDP 发 / 收：").append(udpPacketsOut).append(" / ").append(udpPacketsIn);
        text.append("\nTCP 字节发 / 收：").append(tcpBytesOut.get()).append(" / ").append(tcpBytesIn.get());
        text.append("\nTUN 发 / 收：").append(tunnelTxPackets).append(" / ").append(tunnelRxPackets)
                .append(" · SOCKS 接入 / 认证：").append(socksAccepted).append(" / ").append(socksAuthenticated)
                .append("\nSOCKS 拒绝：").append(socksRejected).append(" · UDP 丢弃：").append(udpDropped)
                .append(" · 最近失败阶段：").append(proxyFailure);
        if (tlsHosts.isEmpty()) return text.append("\n尚未观察到新的 TLS 握手；已建立的连接不会补出登录请求。").toString();
        text.append("\nTLS 主机（仅本次内存；不含路径、参数和内容）：");
        tlsHosts.forEach((host, count) -> text.append('\n').append(host).append(" × ").append(count).append(tlsReplies.contains(host) ? " · 有上游响应" : " · 尚未观察响应"));
        return text.toString();
    }
    public static volatile String status = "准备就绪";
    public static volatile String monitorDetail = "尚未开始检查";
    public static volatile long monitorLastCheckedAt;
    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static final CopyOnWriteArraySet<Runnable> listeners = new CopyOnWriteArraySet<>();
    private static final ArrayDeque<String> logs = new ArrayDeque<>();
    private static Capture capture;
    private AppState() {}
    static void init(Context context) {
        settings = new SettingsStore(context);
        capturePlatform = settings.capturePlatform();
        log("隐私", "不读取聊天、联系人或存储文件；Code 仅驻留内存，5 分钟后清除");
    }
    public record Capture(String code, String ownGid, List<String> gids, long createdAt, boolean friendsObserved, long generation, CapturePlatform platform) {
        private static final java.util.concurrent.atomic.AtomicLong NEXT = new java.util.concurrent.atomic.AtomicLong();
        public Capture(String code, String ownGid, List<String> gids, long createdAt, boolean friendsObserved) {
            this(code, ownGid, gids, createdAt, friendsObserved, NEXT.incrementAndGet());
        }
        public Capture(String code, String ownGid, List<String> gids, long createdAt, boolean friendsObserved, long generation) {
            this(code, ownGid, gids, createdAt, friendsObserved, generation, CapturePlatform.QQ);
        }
        public Capture { java.util.Objects.requireNonNull(platform); gids = java.util.Collections.unmodifiableList(new ArrayList<>(gids)); }
        Capture forPlatform(CapturePlatform source) {
            return new Capture(code, ownGid, gids, createdAt, friendsObserved, generation, source);
        }
        Capture withoutCode() { return new Capture("", ownGid, gids, createdAt, friendsObserved, generation, platform); }
    }
    public static synchronized Capture capture() {
        if (capture != null && SystemClock.elapsedRealtime() - capture.createdAt > 300_000) capture = null;
        return capture;
    }
    public static synchronized void captured(String code, String own, List<String> gids, boolean observed, CapturePlatform platform) {
        captured(new Capture(code, own == null ? "" : own, gids, SystemClock.elapsedRealtime(), observed).forPlatform(platform));
    }
    static synchronized void captured(Capture value) {
        capture = value;
        MAIN.postDelayed(() -> { capture(); changed(); }, Math.max(0, 300_100 - (SystemClock.elapsedRealtime() - value.createdAt)));
        changed();
    }
    static synchronized boolean completeCapture(Capture value) {
        Capture previous = capture();
        // A delayed friend reply must not restore an expired, cleared, replaced,
        // or already submitted Code, nor restart its five-minute lifetime.
        if (previous == null || previous.generation != value.generation || previous.platform != value.platform) return false;
        capture = new Capture(previous.code.isEmpty() ? "" : value.code, value.ownGid, value.gids,
                previous.createdAt, value.friendsObserved, previous.generation, previous.platform);
        changed(); return true;
    }
    public static synchronized void clearCapture() { capture = null; changed(); }
    public static synchronized void consumeCode(Capture original) {
        if (capture != null && capture.generation == original.generation)
            capture = new Capture("", capture.ownGid, capture.gids, capture.createdAt, capture.friendsObserved, capture.generation, capture.platform);
        changed();
    }
    public static synchronized void consumeFriends(Capture original) {
        if (capture != null && capture.generation == original.generation)
            capture = new Capture(capture.code, capture.ownGid, java.util.Collections.emptyList(), capture.createdAt, false, capture.generation, capture.platform);
        changed();
    }
    public static synchronized void log(String tag, String text) {
        // Call sites use fixed messages. Defense in depth for input accidentally passed to logs.
        String safe = text.replaceAll("(?i)((?:code|token|secret|authorization|cookie|open_?id|gid)\\s*[=:]\\s*)[^\\s&,;]+", "$1[已隐藏]")
                .replaceAll("https?://[^\\s]+", "[地址已隐藏]");
        if (safe.length() > 400) safe = safe.substring(0, 400);
        logs.addLast(LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm:ss")) + "  " + tag + "  " + safe);
        while (logs.size() > 200) logs.removeFirst();
        changed();
    }
    public static synchronized String logText() { return String.join("\n", new ArrayList<>(logs)); }
    public static synchronized void clearLogs() { logs.clear(); changed(); }
    public static void listen(Runnable callback) { listeners.add(callback); }
    public static void unlisten(Runnable callback) { listeners.remove(callback); }
    public static void changed() { MAIN.post(() -> { for (Runnable listener : listeners) listener.run(); }); }
}
