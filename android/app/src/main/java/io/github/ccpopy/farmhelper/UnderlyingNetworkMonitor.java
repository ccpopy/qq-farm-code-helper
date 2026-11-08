package io.github.ccpopy.farmhelper;

import android.net.ConnectivityManager;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkRequest;
import android.os.Build;
import android.os.Handler;
import java.net.Inet6Address;
import java.net.InetAddress;
import java.util.List;
import java.util.function.Consumer;

/** Tracks the preferred Internet network, not an IMS network or our own VPN. */
final class UnderlyingNetworkMonitor implements AutoCloseable {
    record Profile(Network network, boolean ipv6, List<InetAddress> dns, int mtu, boolean metered) {}
    private final ConnectivityManager connectivity;
    private final Consumer<Profile> changed;
    private volatile Profile current;
    private boolean closed;
    private final ConnectivityManager.NetworkCallback callback = new ConnectivityManager.NetworkCallback() {
        @Override public void onAvailable(Network network) { update(network); }
        @Override public void onLinkPropertiesChanged(Network network, LinkProperties properties) { update(network); }
        @Override public void onCapabilitiesChanged(Network network, NetworkCapabilities capabilities) { update(network); }
        @Override public void onLost(Network network) {
            Profile previous = current;
            if (previous != null && previous.network().equals(network)) { current = null; changed.accept(null); }
        }
    };
    UnderlyingNetworkMonitor(ConnectivityManager connectivity, Handler handler, Consumer<Profile> changed) {
        this.connectivity = connectivity; this.changed = changed;
        update(connectivity.getActiveNetwork());
        if (Build.VERSION.SDK_INT >= 31) {
            NetworkRequest request = new NetworkRequest.Builder()
                    .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                    .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN).build();
            connectivity.registerBestMatchingNetworkCallback(request, callback, handler);
        } else {
            // The production VPN excludes this app, so its default remains the
            // system's underlying default. The debug self-test ignores its VPN.
            connectivity.registerDefaultNetworkCallback(callback, handler);
        }
    }
    Profile current() { return current; }
    Network network() { Profile profile = current; return profile == null ? null : profile.network(); }
    private void update(Network network) {
        if (closed || network == null) return;
        NetworkCapabilities capabilities = connectivity.getNetworkCapabilities(network);
        LinkProperties properties = connectivity.getLinkProperties(network);
        if (capabilities == null || properties == null || capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)
                || !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)) return;
        boolean route = properties.getRoutes().stream().anyMatch(value -> value.isDefaultRoute()
                && value.getDestination().getAddress() instanceof Inet6Address);
        boolean address = properties.getLinkAddresses().stream().anyMatch(value -> value.getAddress() instanceof Inet6Address
                && !value.getAddress().isLinkLocalAddress() && !value.getAddress().isLoopbackAddress()
                && !value.getAddress().isAnyLocalAddress() && !value.getAddress().isMulticastAddress());
        int mtu = properties.getMtu() > 0 ? Math.max(1280, Math.min(1500, properties.getMtu())) : 1500;
        Profile next = new Profile(network, route && address, List.copyOf(properties.getDnsServers()), mtu,
                !capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED));
        if (!next.equals(current)) { current = next; changed.accept(next); }
    }
    @Override public void close() { closed = true; connectivity.unregisterNetworkCallback(callback); current = null; }
}
