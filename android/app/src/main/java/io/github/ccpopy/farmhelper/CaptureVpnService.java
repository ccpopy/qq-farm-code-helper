package io.github.ccpopy.farmhelper;

import android.app.PendingIntent;
import android.content.Intent;
import android.net.VpnService;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.system.OsConstants;
import java.net.InetAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.SecureRandom;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import hev.htproxy.TProxyService;

public final class CaptureVpnService extends VpnService {
    private static final ExecutorService ENGINE = Executors.newSingleThreadExecutor();
    private final Handler main = new Handler(Looper.getMainLooper());
    private volatile boolean active;
    private boolean stopping;
    private boolean coreConfirmed;
    private volatile int generation;
    private volatile UnderlyingNetworkMonitor networks;
    private UnderlyingNetworkMonitor.Profile configuredNetwork;
    private CertificateManager certificates;
    private String password;
    private LocalSocksProxy proxy;
    private ParcelFileDescriptor tunnel;
    private CapturePlatform platform;
    private boolean captureFriends;
    private long notifiedCapture;
    private int recoveryAttempt, recoveryToken;
    private boolean recoveryScheduled;
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || "STOP".equals(intent.getAction())) { stopCapture(); return START_NOT_STICKY; }
        if (active || stopping) return START_NOT_STICKY;
        OverlayService.sync(this);
        platform = AppState.settings.capturePlatform();
        captureFriends = AppState.settings.friendCapture();
        AppState.capturePlatform = platform;
        AppState.capturePassthrough = AppState.settings.passthrough();
        startForeground(11, Notifications.ongoing(this, CaptureVpnService.class, platform.label + (AppState.capturePassthrough ? " 全量透传诊断" : " 农场获取"),
                "仅 " + platform.label + " · " + (AppState.capturePassthrough ? "不解密、不获取凭据" : "仅解析农场目标 TLS") + " · 可随时停止", 11));
        active = true; AppState.capturing = true; AppState.captureReady = false; AppState.tlsVerified = false;
        AppState.gatewayConnections = AppState.gatewayTlsAttempts = 0;
        AppState.socksAccepted = AppState.socksAuthenticated = AppState.socksRejected = 0;
        AppState.tunnelTxPackets = AppState.tunnelRxPackets = 0;
        AppState.proxyFailure = "none";
        AppState.resetConnections();
        AppState.clearCapture(); AppState.status = "正在准备 VPN"; AppState.changed();
        ENGINE.execute(this::prepare);
        return START_NOT_STICKY;
    }
    private void prepare() {
        try {
            getPackageManager().getApplicationInfo(platform.packageName, 0);
            certificates = AppState.capturePassthrough ? null : new CertificateManager(this);
            if (certificates != null) certificates.serverContext(); // Validate local CA/key before starting the VPN.
            var connectivity = getSystemService(android.net.ConnectivityManager.class);
            networks = new UnderlyingNetworkMonitor(connectivity, main, profile -> ENGINE.execute(() -> networkChanged(profile)));
            byte[] random = new byte[24]; new SecureRandom().nextBytes(random);
            password = android.util.Base64.encodeToString(random, android.util.Base64.NO_WRAP);
            if (!active) return;
            var profile = networks.current();
            if (profile == null) {
                AppState.status = "等待底层网络恢复"; AppState.changed(); return;
            }
            configure(profile);
        } catch (Throwable error) {
            if (active && password != null && networks != null) { scheduleRecovery("VPN 转发准备尚未完成"); return; }
            AppState.status = "VPN 未能启动";
            AppState.log("VPN", "准备失败：请检查 " + platform.label + " 是否安装、本地证书及网络；不会修改系统证书");
            stopCapture();
        }
    }
    private void networkChanged(UnderlyingNetworkMonitor.Profile profile) {
        if (!active || password == null || profile != null && profile.equals(configuredNetwork)) return;
        if (profile == null) {
            configuredNetwork = null;
            if (proxy != null) proxy.networkChanged();
            AppState.captureReady = false;
            AppState.status = "等待底层网络恢复"; AppState.changed(); return;
        }
        try {
            AppState.log("VPN", "底层网络已变化，重新配置路由、DNS 和受保护连接；无需重启 " + platform.label);
            AppState.captureReady = false; AppState.changed();
            closeForwarding(true);
            configure(profile);
        } catch (Throwable error) {
            scheduleRecovery("网络切换后转发重建未完成");
        }
    }
    private void configure(UnderlyingNetworkMonitor.Profile profile) throws Exception {
            if (!active) return;
            recoveryScheduled = false; recoveryToken++;
            configuredNetwork = profile; coreConfirmed = false;
            int currentGeneration = ++generation;
            AppState.vpnIpv6 = profile.ipv6();
            AppState.log("VPN", "当前底层网络：IPv4" + (profile.ipv6() ? " / IPv6 双栈" : "；未检测到可用 IPv6")
                    + "，DNS 数量：" + profile.dns().size() + "，MTU：" + profile.mtu());
            proxy = new LocalSocksProxy(this, () -> networks == null ? null : networks.network(), certificates, password, captureFriends, AppState.capturePassthrough,
                    () -> ENGINE.execute(() -> scheduleRecovery("UDP 转发需要重建")), (rawCapture, complete) -> main.post(() -> {
                if (!active || currentGeneration != generation) return;
                // WeChat Codes are single-use. Friend inspection forwards the login;
                // never present that already-forwarded Code as reusable credentials.
                boolean friendsOnly = platform == CapturePlatform.WECHAT && captureFriends;
                AppState.Capture captured = (friendsOnly ? rawCapture.withoutCode() : rawCapture).forPlatform(platform);
                if (!AppState.networkCaptured(captured, complete)) return;
                if (notifiedCapture != captured.generation() && AppState.hasPendingCapture()
                        && (complete || captured.friendsObserved() || !captured.ownGid().isEmpty())) {
                    notifiedCapture = captured.generation();
                    AppState.log("抓取", "已取得可同步结果；可立即推送，VPN 将继续接收好友，清除结果可换号获取");
                    Notifications.alert(this, "", friendsOnly ? "好友已读取" : "农场凭据已获取", "可立即确认账号并推送；VPN 保持运行。");
                }
            }));
            Builder builder = new Builder().setSession("农场助手 · 仅 " + platform.label + " 全量接管").setMtu(profile.mtu()).setMetered(profile.metered())
                    .addAddress("10.231.0.1", 32).allowFamily(OsConstants.AF_INET)
                    .addAllowedApplication(platform.packageName)
                    .setConfigureIntent(PendingIntent.getActivity(this, 1, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
            builder.addRoute("0.0.0.0", 0);
            if (AppState.vpnIpv6) builder.addAddress("fd71:fa42::1", 128).addRoute("::", 0);
            for (InetAddress dns : profile.dns()) builder.addDnsServer(dns);
            builder.setUnderlyingNetworks(new android.net.Network[]{profile.network()});
            String config = "tunnel:\n  mtu: " + profile.mtu() + "\n  ipv4: 10.231.0.1\n  ipv6: 'fd71:fa42::1'\nsocks5:\n  address: 127.0.0.1\n  port: " + proxy.port()
                    + "\n  username: farm\n  password: '" + password + "'\n  udp: udp\nmisc:\n  log-level: error\n  log-file: null\n  max-session-count: 0\n  connect-timeout: 10000\n  tcp-read-write-timeout: 0\n  udp-read-write-timeout: 0\n";
            var path = new java.io.File(getFilesDir(), "tunnel.yml").toPath();
            Files.write(path, config.getBytes(StandardCharsets.UTF_8));
            if (!active) return;
            ParcelFileDescriptor replacement = builder.establish();
            if (replacement == null) throw new IllegalStateException("VPN 授权已撤销");
            ParcelFileDescriptor previous = tunnel;
            tunnel = replacement;
            if (previous != null) try { previous.close(); } catch (Exception ignored) { }
            if (!TProxyService.TProxyStartService(path.toString(), tunnel.getFd())) throw new IllegalStateException("VPN 转发核心启动失败");
            if (!active) return;
            main.postDelayed(() -> ENGINE.execute(() -> checkCore(currentGeneration, 3)), 350);
    }
    private void checkCore(int expectedGeneration, int startupRetries) {
        if (!active || expectedGeneration != generation) return;
        if (!TProxyService.TProxyIsRunning()) {
            if (startupRetries > 0) { main.postDelayed(() -> ENGINE.execute(() -> checkCore(expectedGeneration, startupRetries - 1)), 350); return; }
            scheduleRecovery("VPN 转发核心需要恢复"); return;
        }
        recoveryAttempt = 0;
        sampleTraffic();
        AppState.changed();
        if (!coreConfirmed) {
            coreConfirmed = true;
            AppState.captureReady = true;
            AppState.status = AppState.capturePassthrough ? platform.label + " 全量透传就绪 · 不获取凭据" : AppState.captureSummary();
            AppState.log("VPN", "转发已就绪：按 " + platform.label + " 应用接管全部 TCP/UDP，不按 IP/域名筛选入口；"
                    + (AppState.capturePassthrough ? "本次仅透传，TLS 不解密，不获取凭据" : "仅农场目标 SNI 解密，其他内容原样透传")); AppState.changed();
        }
        main.postDelayed(() -> ENGINE.execute(() -> checkCore(expectedGeneration, 0)), 2000);
    }
    /** Transient forwarding failures rebuild the running session, never consume results. */
    private void scheduleRecovery(String reason) {
        if (!active || recoveryScheduled) return;
        recoveryScheduled = true;
        AppState.captureReady = false; AppState.captureCollecting = false;
        int token = ++recoveryToken;
        long delay = Math.min(30_000L, 1000L << Math.min(recoveryAttempt++, 5));
        AppState.status = "VPN 转发正在恢复 · 已有结果仍可同步";
        AppState.log("VPN", reason + "，将自动重试；无需停止获取或清除结果"); AppState.changed();
        main.postDelayed(() -> ENGINE.execute(() -> {
            if (!active || token != recoveryToken) return;
            recoveryScheduled = false;
            var profile = networks == null ? null : networks.current();
            if (profile == null) {
                configuredNetwork = null;
                AppState.status = "等待底层网络恢复 · 已有结果仍可同步"; AppState.changed(); return;
            }
            closeForwarding(true);
            try { configure(profile); }
            catch (Throwable failure) { scheduleRecovery("VPN 转发重建尚未成功"); }
        }), delay);
    }
    static void sampleTraffic() {
        long[] traffic = TProxyService.TProxyGetStats();
        if (traffic != null && traffic.length == 4) {
            AppState.tunnelTxPackets = traffic[0]; AppState.tunnelRxPackets = traffic[2];
        }
    }
    @Override public void onRevoke() {
        AppState.status = "VPN 已被系统撤销或其他 VPN 接管";
        stopCapture();
    }
    private void closeForwarding() { closeForwarding(false); }
    private void closeForwarding(boolean keepTunnel) {
        ++generation;
        try { sampleTraffic(); } catch (Throwable ignored) { }
        try { TProxyService.TProxyStopService(); } catch (Throwable ignored) { }
        if (proxy != null) { proxy.close(); proxy = null; }
        AppState.networkForwardingClosed();
        if (!keepTunnel && tunnel != null) { try { tunnel.close(); } catch (Exception ignored) { } tunnel = null; }
        AppState.captureCollecting = false;
    }
    // Android binds to an established VPN. stopSelf()/stopService() alone may
    // leave that binding alive, so onDestroy() cannot be our shutdown trigger.
    // Close the tunnel first; only then report stopped and stop the service.
    void stopCapture() {
        if (Looper.myLooper() != Looper.getMainLooper()) { main.post(this::stopCapture); return; }
        if (stopping) return;
        stopping = true;
        active = false; main.removeCallbacksAndMessages(null);
        AppState.captureReady = false; AppState.captureCollecting = false;
        ENGINE.execute(() -> {
            try {
                if (networks != null) { networks.close(); networks = null; }
            } finally {
                closeForwarding();
                configuredNetwork = null; password = null; certificates = null;
                try { Files.deleteIfExists(new java.io.File(getFilesDir(), "tunnel.yml").toPath()); } catch (Exception ignored) { }
                main.post(() -> {
                    AppState.capturing = false;
                    if (!AppState.status.equals("VPN 未能启动") && !AppState.status.equals("VPN 已被系统撤销或其他 VPN 接管"))
                        AppState.status = AppState.capture() == null ? "VPN 已停止" : AppState.captureSummary();
                    AppState.log("VPN", "已停止抓取并释放网络；已安装的系统证书不会被自动删除");
                    stopForeground(STOP_FOREGROUND_REMOVE);
                    stopSelf();
                });
            }
        });
    }
    @Override public void onDestroy() {
        stopCapture();
        super.onDestroy();
    }
}
