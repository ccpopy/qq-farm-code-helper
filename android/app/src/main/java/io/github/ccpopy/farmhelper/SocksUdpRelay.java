package io.github.ccpopy.farmhelper;

import android.net.VpnService;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.ByteBuffer;
import java.nio.channels.DatagramChannel;
import java.nio.channels.SelectionKey;
import java.nio.channels.Selector;
import java.util.HashMap;
import java.util.concurrent.ConcurrentLinkedQueue;

/** One selector thread for all protected UDP flows, including DNS and QUIC. No payload logs. */
final class SocksUdpRelay implements AutoCloseable {
    private final VpnService vpn;
    private final java.util.function.Supplier<android.net.Network> network;
    private final Selector selector;
    private final Thread thread;
    private final ConcurrentLinkedQueue<Runnable> commands = new ConcurrentLinkedQueue<>();
    private final HashMap<FlowKey, Flow> flows = new HashMap<>();
    private volatile boolean active = true;
    private record FlowKey(Association association, InetSocketAddress destination) {}
    private static final class Association {
        final DatagramChannel channel;
        volatile boolean open = true;
        InetSocketAddress client;
        Association(DatagramChannel channel) { this.channel = channel; }
    }
    private static final class Flow {
        final FlowKey key;
        final DatagramChannel channel;
        Flow(FlowKey key, DatagramChannel channel) { this.key = key; this.channel = channel; }
    }
    private final Runnable stopCapture;
    SocksUdpRelay(VpnService vpn, java.util.function.Supplier<android.net.Network> network, Runnable stopCapture) throws java.io.IOException {
        this.vpn = vpn; this.network = network; this.stopCapture = stopCapture;
        selector = Selector.open(); thread = new Thread(this::loop, "farm-udp"); thread.start();
    }
    void serve(Socket control, InputStream input, OutputStream output) throws Exception {
        DatagramChannel channel = DatagramChannel.open();
        Association association = new Association(channel);
        try {
            channel.bind(new InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0)); channel.configureBlocking(false);
            enqueue(() -> { try { channel.register(selector, SelectionKey.OP_READ, association); } catch (Exception ignored) { closeChannel(channel); } });
            int port = ((InetSocketAddress)channel.getLocalAddress()).getPort();
            output.write(new byte[]{5, 0, 0, 1, 127, 0, 0, 1, (byte)(port >>> 8), (byte)port}); output.flush();
            AppState.udpAssociations++;
            control.setSoTimeout(0);
            while (active && input.read() != -1) { /* RFC 1928 control connection stays open. */ }
        } finally {
            association.open = false; closeChannel(channel);
            enqueue(() -> flows.entrySet().removeIf(entry -> {
                if (entry.getKey().association() != association) return false;
                closeChannel(entry.getValue().channel); return true;
            }));
        }
    }
    private void enqueue(Runnable command) { commands.add(command); selector.wakeup(); }
    private void loop() {
        byte[] bytes = new byte[65536]; ByteBuffer buffer = ByteBuffer.wrap(bytes);
        try {
            while (active) {
                Runnable command; while ((command = commands.poll()) != null) command.run();
                selector.select(250);
                var selected = selector.selectedKeys().iterator();
                while (selected.hasNext()) {
                    SelectionKey key = selected.next(); selected.remove();
                    if (!key.isValid() || !key.isReadable()) continue;
                    try {
                        buffer.clear();
                        if (key.attachment() instanceof Association association) {
                            var sender = (InetSocketAddress)association.channel.receive(buffer);
                            if (sender == null || !sender.getAddress().isLoopbackAddress() || !association.open) continue;
                            if (association.client == null) association.client = sender;
                            if (!association.client.equals(sender)) { AppState.udpDropped++; continue; }
                            SocksDatagram.Frame packet = SocksDatagram.decode(bytes, buffer.position());
                            FlowKey destination = new FlowKey(association, packet.target());
                            Flow flow = flows.get(destination);
                            if (flow == null) {
                                DatagramChannel upstream = DatagramChannel.open();
                                try {
                                    upstream.bind(new InetSocketAddress(InetAddress.getByAddress(new byte[packet.target().getAddress().getAddress().length]), 0));
                                    if (!vpn.protect(upstream.socket())) throw new java.io.IOException("UDP socket protection failed");
                                    android.net.Network selectedNetwork = network.get();
                                    if (selectedNetwork == null) throw new java.net.NoRouteToHostException("No underlying network");
                                    selectedNetwork.bindSocket(upstream.socket());
                                    upstream.connect(packet.target()); upstream.configureBlocking(false);
                                    flow = new Flow(destination, upstream); upstream.register(selector, SelectionKey.OP_READ, flow);
                                    flows.put(destination, flow);
                                } catch (Exception error) { closeChannel(upstream); throw error; }
                            }
                            if (flow.channel.write(ByteBuffer.wrap(bytes, packet.offset(), packet.length())) == packet.length()) AppState.udpPacketsOut++;
                            else AppState.udpDropped++;
                        } else if (key.attachment() instanceof Flow flow) {
                            int length = flow.channel.read(buffer);
                            Association association = flow.key.association();
                            if (length < 0 || !association.open || association.client == null) continue;
                            byte[] response = SocksDatagram.encode(flow.key.destination(), bytes, length);
                            if (association.channel.send(ByteBuffer.wrap(response), association.client) == response.length) AppState.udpPacketsIn++;
                            else AppState.udpDropped++;
                        }
                    } catch (Exception error) {
                        AppState.udpDropped++;
                        if (key.attachment() instanceof Flow flow) { closeChannel(flow.channel); flows.remove(flow.key); }
                    }
                }
                // The authenticated SOCKS control connection owns each mapping.
                // Its closure, a network change, or VPN shutdown releases flows.
            }
        } catch (Exception error) { if (active) { AppState.log("代理", "UDP 转发停止，正在停止本次 VPN 以恢复网络"); stopCapture.run(); } }
        finally {
            java.util.Arrays.fill(bytes, (byte)0);
            for (SelectionKey key : selector.keys()) closeChannel(key.channel());
            try { selector.close(); } catch (Exception ignored) { }
            flows.clear(); commands.clear();
        }
    }
    private static void closeChannel(java.nio.channels.Channel channel) { try { channel.close(); } catch (Exception ignored) { } }
    void networkChanged() {
        enqueue(() -> {
            for (Flow flow : flows.values()) closeChannel(flow.channel);
            flows.clear();
        });
    }
    @Override public void close() {
        active = false; selector.wakeup();
        try { thread.join(1500); } catch (InterruptedException error) { Thread.currentThread().interrupt(); }
    }
}
