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
    /** The companion window stands down while the user is already looking at the app. */
    public static volatile boolean appVisible;
    public static volatile boolean captureReady;
    /** Receiving this round's replies; available results can still be submitted. */
    public static volatile boolean captureCollecting;
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
    private static long captureEpoch;
    private static long networkCaptureGeneration;
    private static boolean networkCapturePublished;
    private static boolean captureCodeExpired;
    private static final java.util.LinkedHashSet<String> consumedGids = new java.util.LinkedHashSet<>();
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
        if (capture != null && !capture.code.isEmpty() && SystemClock.elapsedRealtime() - capture.createdAt >= 300_000) {
            capture = capture.withoutCode();
            captureCodeExpired = true;
            status = captureSummary();
        }
        return capture;
    }
    public static synchronized void captured(String code, String own, List<String> gids, boolean observed, CapturePlatform platform) {
        captured(new Capture(code, own == null ? "" : own, gids, SystemClock.elapsedRealtime(), observed).forPlatform(platform));
    }
    static synchronized void captured(Capture value) {
        resetCapture();
        saveCapture(value);
        changed();
    }
    private static void saveCapture(Capture value) {
        capture = value;
        MAIN.postDelayed(() -> { capture(); changed(); }, Math.max(0, 300_100 - (SystemClock.elapsedRealtime() - value.createdAt)));
    }
    /** A TCP connection belongs to the round in which it was accepted. */
    static synchronized long captureEpoch() { capture(); return captureEpoch; }
    static synchronized boolean claimNetworkCapture(long expectedEpoch, long generation) {
        capture();
        if (!capturing || expectedEpoch != captureEpoch || networkCaptureGeneration != 0 || capture != null) return false;
        networkCaptureGeneration = generation;
        return true;
    }
    static synchronized boolean acceptsNetworkCapture(long generation) {
        capture();
        return capturing && networkCaptureGeneration == generation;
    }
    static synchronized void releaseUnpublishedCapture(long generation) {
        if (!networkCapturePublished && networkCaptureGeneration == generation) resetCapture();
    }
    static synchronized void releaseUnpublishedCapture() {
        if (!networkCapturePublished && networkCaptureGeneration != 0) resetCapture();
    }
    static synchronized void networkForwardingClosed() {
        // Called after the old proxy is closed. Its already accepted sockets
        // cannot claim the next round while their worker threads unwind.
        captureEpoch++;
        if (!networkCapturePublished) networkCaptureGeneration = 0;
        captureCollecting = false;
    }
    static synchronized boolean networkCaptured(Capture value, boolean complete) {
        if (!acceptsNetworkCapture(value.generation)) return false;
        if (!networkCapturePublished) {
            networkCapturePublished = true;
            saveCapture(value);
            capture();
        } else if (!completeCapture(value)) return false;
        captureCollecting = !complete;
        status = captureSummary();
        changed();
        return true;
    }
    static synchronized boolean completeCapture(Capture value) {
        Capture previous = capture();
        // A delayed friend reply must not restore an expired, cleared, replaced,
        // or already submitted Code, nor restart its five-minute lifetime.
        if (previous == null || previous.generation != value.generation || previous.platform != value.platform) return false;
        if (!previous.ownGid.isEmpty() && !value.ownGid.isEmpty() && !previous.ownGid.equals(value.ownGid)) return false;
        String own = value.ownGid.isEmpty() ? previous.ownGid : value.ownGid;
        java.util.LinkedHashSet<String> pending = new java.util.LinkedHashSet<>(previous.gids);
        pending.addAll(value.gids); pending.removeAll(consumedGids); pending.remove(own);
        capture = new Capture(previous.code, own, new ArrayList<>(pending),
                previous.createdAt, previous.friendsObserved || value.friendsObserved, previous.generation, previous.platform);
        changed(); return true;
    }
    private static void resetCapture() {
        capture = null; captureEpoch++; networkCaptureGeneration = 0; networkCapturePublished = false;
        captureCollecting = false; captureCodeExpired = false; consumedGids.clear();
    }
    public static synchronized void clearCapture() {
        resetCapture();
        if (capturing) status = "结果已清除 · 可换号重新进入农场";
        changed();
    }
    /** Incremental replies keep a selected snapshot valid until its data is consumed. */
    public static synchronized boolean isCurrentCapture(Capture original) {
        Capture current = capture();
        return original != null && current != null && original.generation == current.generation && original.platform == current.platform
                && (original.code.isEmpty() || original.code.equals(current.code))
                && current.gids.containsAll(original.gids)
                && (original.ownGid.isEmpty() || current.ownGid.isEmpty() || original.ownGid.equals(current.ownGid));
    }
    public static synchronized boolean hasPendingCapture() {
        Capture current = capture();
        return current != null && (!current.code.isEmpty() || !current.gids.isEmpty());
    }
    public static synchronized String captureSummary() {
        Capture current = capture();
        if (current == null) return capturing ? "等待进入 " + capturePlatform.label + " 农场" : "暂无获取结果";
        String suffix = capturing ? " · VPN 保持运行" : "";
        if (!current.gids.isEmpty()) return (captureCodeExpired ? "Code 已过期 · " : current.code.isEmpty() ? "" : "Code 已取得 · ") + "已读取 " + current.gids.size() + " 位好友，可立即同步" + suffix;
        if (!current.code.isEmpty()) return "Code 已取得，可立即同步" + (captureCollecting ? " · 等待好友回包" : "") + suffix;
        if (!consumedGids.isEmpty()) return (captureCodeExpired ? "Code 已过期 · " : "") + "已同步当前好友" + (captureCollecting ? " · 继续接收新好友" : "") + suffix;
        if (current.friendsObserved) return (captureCodeExpired ? "Code 已过期 · " : "") + "已收到好友回包 · 暂无待同步好友" + suffix;
        if (captureCodeExpired) return "Code 已过期，请清除后重新获取" + (captureCollecting ? " · 仍可接收好友" : "") + suffix;
        return (captureCollecting ? "农场已连接 · 等待好友回包" : "当前结果已同步") + suffix;
    }
    public static synchronized void consumeCode(Capture original) {
        if (capture != null && capture.generation == original.generation && capture.platform == original.platform)
            capture = new Capture("", capture.ownGid, capture.gids, capture.createdAt, capture.friendsObserved, capture.generation, capture.platform);
        status = captureSummary();
        changed();
    }
    public static synchronized void consumeFriends(Capture original) {
        if (capture != null && capture.generation == original.generation && capture.platform == original.platform) {
            consumedGids.addAll(original.gids);
            ArrayList<String> pending = new ArrayList<>(capture.gids); pending.removeAll(consumedGids);
            capture = new Capture(capture.code, capture.ownGid, pending, capture.createdAt, capture.friendsObserved, capture.generation, capture.platform);
        }
        status = captureSummary();
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
