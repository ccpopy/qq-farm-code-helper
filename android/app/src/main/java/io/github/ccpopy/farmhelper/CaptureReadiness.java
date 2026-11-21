package io.github.ccpopy.farmhelper;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;

/** Shared start checks for the main screen and floating window. */
record CaptureReadiness(boolean passthrough, boolean generated, boolean installed, int servers, String networkHint) {
    static CaptureReadiness check(Context context) {
        boolean passthrough = AppState.settings.passthrough();
        CertificateManager certificate = new CertificateManager(context);
        boolean generated = !passthrough && certificate.exists();
        boolean installed = false;
        if (generated) {
            try { installed = certificate.visibleInSystem(); }
            catch (Exception unavailable) { /* The user can confirm installation when the store cannot be read. */ }
        }
        return new CaptureReadiness(passthrough, generated, installed, AppState.settings.readyServers().size(),
                networkHint(context.getSystemService(ConnectivityManager.class)));
    }

    boolean certificateReady() { return passthrough || generated && installed; }
    boolean needsChecklist() { return !certificateReady() || servers == 0 || !networkHint.isEmpty(); }
    int missingCertificateSteps() { return passthrough ? 0 : !generated ? 2 : installed ? 0 : 1; }

    private static String networkHint(ConnectivityManager connectivity) {
        if (connectivity == null) return "当前网络状态无法读取，可在系统设置中检查。";
        Network network = connectivity.getActiveNetwork();
        NetworkCapabilities active = network == null ? null : connectivity.getNetworkCapabilities(network);
        if (active == null || !active.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) {
            return "当前网络尚未连通互联网，可检查 WLAN 或移动数据。";
        }
        if (active.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
            for (Network candidate : connectivity.getAllNetworks()) {
                NetworkCapabilities capabilities = connectivity.getNetworkCapabilities(candidate);
                if (capabilities != null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
                        && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                        && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
                        && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_SUSPENDED)) {
                    return "WLAN 与移动数据同时开启。若进入农场超时，请关闭移动数据或系统的“网络加速 / 双通道加速”后重试。";
                }
            }
        }
        return "";
    }
}
