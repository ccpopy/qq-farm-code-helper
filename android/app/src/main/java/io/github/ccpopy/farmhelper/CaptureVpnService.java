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
    private boolean coreConfirmed;
    private int generation;
    private volatile UnderlyingNetworkMonitor networks;
    private UnderlyingNetworkMonitor.Profile configuredNetwork;
    private javax.net.ssl.SSLContext tls;
    private String password;
    private LocalSocksProxy proxy;
    private ParcelFileDescriptor tunnel;
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || "STOP".equals(intent.getAction())) { stopSelf(); return START_NOT_STICKY; }
        if (active) return START_NOT_STICKY;
        AppState.capturePassthrough = AppState.settings.passthrough();
        startForeground(11, Notifications.ongoing(this, CaptureVpnService.class, AppState.capturePassthrough ? "QQ 全量透传诊断" : "QQ 农场获取",
                "仅 QQ · " + (AppState.capturePassthrough ? "不解密、不获取凭据" : "仅解析农场目标 TLS") + " · 2 分钟自动停止", 11));
        active = true; AppState.capturing = true; AppState.captureReady = false; AppState.tlsVerified = false;
        AppState.gatewayConnections = AppState.gatewayTlsAttempts = 0;
        AppState.socksAccepted = AppState.socksAuthenticated = AppState.socksRejected = 0;
        AppState.tunnelTxPackets = AppState.tunnelRxPackets = 0;
        AppState.proxyFailure = "none";
        AppState.resetConnections();
        AppState.clearCapture(); AppState.status = "正在准备 VPN"; AppState.changed();
        ENGINE.execute(this::prepare);
        main.postDelayed(() -> { if (active) { AppState.log("VPN", "已到 2 分钟抓取时限，自动恢复网络"); stopSelf(); } }, 120_000);
        return START_NOT_STICKY;
    }
    private void prepare() {
        try {
            getPackageManager().getApplicationInfo("com.tencent.mobileqq", 0);
            tls = AppState.capturePassthrough ? null : new CertificateManager(this).serverContext();
            var connectivity = getSystemService(android.net.ConnectivityManager.class);
            networks = new UnderlyingNetworkMonitor(connectivity, main, profile -> ENGINE.execute(() -> networkChanged(profile)));
            byte[] random = new byte[24]; new SecureRandom().nextBytes(random);
            password = android.util.Base64.encodeToString(random, android.util.Base64.NO_WRAP);
            if (!active) return;
            var profile = networks.current();
            if (profile == null) throw new IllegalStateException("没有底层互联网连接");
            configure(profile);
        } catch (Throwable error) {
            AppState.status = "VPN 未能启动";
            AppState.log("VPN", "准备失败：请检查 QQ 是否安装、本地证书及网络；不会修改系统证书");
            main.post(this::stopSelf);
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
            AppState.log("VPN", "底层网络已变化，重新配置路由、DNS 和受保护连接；无需重启 QQ");
            AppState.captureReady = false; AppState.changed();
            closeForwarding();
            configure(profile);
        } catch (Throwable error) {
            AppState.status = "VPN 未能启动";
            AppState.log("VPN", "网络切换后重建失败，已停止 VPN 恢复网络"); main.post(this::stopSelf);
        }
    }
    private void configure(UnderlyingNetworkMonitor.Profile profile) throws Exception {
            if (!active) return;
            configuredNetwork = profile; coreConfirmed = false;
            int currentGeneration = ++generation;
            AppState.vpnIpv6 = profile.ipv6();
            AppState.log("VPN", "当前底层网络：IPv4" + (profile.ipv6() ? " / IPv6 双栈" : "；未检测到可用 IPv6")
                    + "，DNS 数量：" + profile.dns().size() + "，MTU：" + profile.mtu());
            proxy = new LocalSocksProxy(this, () -> networks == null ? null : networks.network(), tls, password, AppState.settings.friendCapture(), AppState.capturePassthrough, captured -> {
                if (!active) return;
                AppState.captured(captured.code(), captured.ownGid(), captured.gids(), captured.friendsObserved());
                AppState.status = captured.friendsObserved() ? "已获取 Code · " + captured.gids().size() + " 位好友" : "已获取 Code · 好友尚未取得";
                AppState.log("抓取", "抓取结束，凭据仅保留在内存；请选择对应账号后推送");
                Notifications.alert(this, AppState.settings.targetId(), "农场凭据已获取", "打开应用确认目标账号并推送。Code 不会自动上传。");
                main.postDelayed(this::stopSelf, 300);
            });
            Builder builder = new Builder().setSession("农场助手 · 仅 QQ 全量接管").setMtu(profile.mtu()).setMetered(profile.metered())
                    .addAddress("10.231.0.1", 32).allowFamily(OsConstants.AF_INET)
                    .addAllowedApplication("com.tencent.mobileqq")
                    .setConfigureIntent(PendingIntent.getActivity(this, 1, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
            builder.addRoute("0.0.0.0", 0);
            if (AppState.vpnIpv6) builder.addAddress("fd71:fa42::1", 128).addRoute("::", 0);
            for (InetAddress dns : profile.dns()) builder.addDnsServer(dns);
            builder.setUnderlyingNetworks(new android.net.Network[]{profile.network()});
            String config = "tunnel:\n  mtu: " + profile.mtu() + "\n  ipv4: 10.231.0.1\n  ipv6: 'fd71:fa42::1'\nsocks5:\n  address: 127.0.0.1\n  port: " + proxy.port()
                    + "\n  username: farm\n  password: '" + password + "'\n  udp: udp\nmisc:\n  log-level: error\n  log-file: null\n  max-session-count: 64\n  connect-timeout: 10000\n";
            var path = new java.io.File(getFilesDir(), "tunnel.yml").toPath();
            Files.write(path, config.getBytes(StandardCharsets.UTF_8));
            if (!active) return;
            tunnel = builder.establish();
            if (tunnel == null) throw new IllegalStateException("VPN 授权已撤销");
            if (!TProxyService.TProxyStartService(path.toString(), tunnel.getFd())) throw new IllegalStateException("VPN 转发核心启动失败");
            if (!active) return;
            main.postDelayed(() -> ENGINE.execute(() -> checkCore(currentGeneration, 3)), 350);
    }
    private void checkCore(int expectedGeneration, int startupRetries) {
        if (!active || expectedGeneration != generation) return;
        if (!TProxyService.TProxyIsRunning()) {
            if (startupRetries > 0) { main.postDelayed(() -> ENGINE.execute(() -> checkCore(expectedGeneration, startupRetries - 1)), 350); return; }
            AppState.status = "VPN 转发核心已退出";
            AppState.log("VPN", "转发核心未保持运行，已停止接管网络；请导出脱敏日志排查兼容性");
            AppState.changed(); main.post(this::stopSelf); return;
        }
        sampleTraffic();
        AppState.changed();
        if (!coreConfirmed && AppState.capture() == null) {
            coreConfirmed = true;
            AppState.captureReady = true;
            AppState.status = AppState.capturePassthrough ? "全量透传就绪 · 不获取凭据" : "等待进入 QQ 农场";
            AppState.log("VPN", "转发已就绪：按 QQ 应用接管全部 TCP/UDP，不按 IP/域名筛选入口；"
                    + (AppState.capturePassthrough ? "本次仅透传，TLS 不解密，不获取凭据" : "仅农场目标 SNI 解密，其他内容原样透传")); AppState.changed();
        }
        main.postDelayed(() -> ENGINE.execute(() -> checkCore(expectedGeneration, 0)), 2000);
    }
    static void sampleTraffic() {
        long[] traffic = TProxyService.TProxyGetStats();
        if (traffic != null && traffic.length == 4) {
            AppState.tunnelTxPackets = traffic[0]; AppState.tunnelRxPackets = traffic[2];
        }
    }
    @Override public void onRevoke() { stopSelf(); }
    private void closeForwarding() {
        ++generation;
        try { sampleTraffic(); } catch (Throwable ignored) { }
        try { TProxyService.TProxyStopService(); } catch (Throwable ignored) { }
        if (proxy != null) { proxy.close(); proxy = null; }
        if (tunnel != null) { try { tunnel.close(); } catch (Exception ignored) { } tunnel = null; }
    }
    @Override public void onDestroy() {
        active = false; main.removeCallbacksAndMessages(null);
        ENGINE.execute(() -> {
            if (networks != null) { networks.close(); networks = null; }
            closeForwarding();
            configuredNetwork = null; password = null;
            try { Files.deleteIfExists(new java.io.File(getFilesDir(), "tunnel.yml").toPath()); } catch (Exception ignored) { }
        });
        AppState.capturing = false; AppState.captureReady = false;
        if (AppState.capture() == null && !AppState.status.equals("VPN 未能启动") && !AppState.status.equals("VPN 转发核心已退出")) AppState.status = "VPN 已停止";
        AppState.log("VPN", "已停止抓取并释放网络；已安装的系统证书不会被自动删除");
        AppState.changed(); stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }
}
