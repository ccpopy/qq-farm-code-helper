package io.github.ccpopy.farmhelper;

import android.net.VpnService;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.Semaphore;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;
import javax.net.ssl.SNIHostName;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLParameters;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.SSLSocketFactory;

/** Full per-QQ forwarding. Passthrough mode never decrypts; capture mode only decrypts the farm SNI. */
public final class LocalSocksProxy implements AutoCloseable {
    private final VpnService vpn;
    private final java.util.function.Supplier<android.net.Network> network;
    private final SSLContext tls;
    private final String password;
    private final boolean captureFriends;
    private final boolean passthrough;
    private final Consumer<AppState.Capture> result;
    private final ServerSocket listener;
    private final Set<Socket> sockets = ConcurrentHashMap.newKeySet();
    private final java.util.concurrent.ThreadPoolExecutor workers = new java.util.concurrent.ThreadPoolExecutor(0, 160, 20,
            TimeUnit.SECONDS, new java.util.concurrent.SynchronousQueue<>());
    private final ScheduledExecutorService deadlines = Executors.newSingleThreadScheduledExecutor();
    private final AtomicBoolean claimed = new AtomicBoolean();
    private final Semaphore slots = new Semaphore(48);
    private final SocksUdpRelay udp;
    private volatile boolean active = true;
    private static final class StageFailure extends IOException {
        final String stage;
        StageFailure(String stage, Exception cause) { super(stage, cause); this.stage = stage; }
    }

    public LocalSocksProxy(VpnService vpn, java.util.function.Supplier<android.net.Network> network, SSLContext tls, String password,
                           boolean captureFriends, boolean passthrough, Consumer<AppState.Capture> result) throws IOException {
        this.vpn = vpn; this.network = network; this.tls = tls; this.password = password;
        this.captureFriends = captureFriends; this.passthrough = passthrough; this.result = result;
        listener = new ServerSocket(); listener.bind(new InetSocketAddress(InetAddress.getByName("127.0.0.1"), 0), 16);
        try { udp = new SocksUdpRelay(vpn, network); } catch (IOException error) { listener.close(); throw error; }
        Thread accept = new Thread(this::accept, "farm-socks"); accept.start();
    }
    public int port() { return listener.getLocalPort(); }
    void networkChanged() { udp.networkChanged(); }
    private void accept() {
        while (active) try {
            Socket socket = listener.accept();
            AppState.socksAccepted++;
            if (!slots.tryAcquire()) { socket.close(); continue; }
            sockets.add(socket);
            try { workers.execute(() -> {
                try { socket.setSoTimeout(15_000); socket.setTcpNoDelay(true); handle(socket); }
                catch (Exception e) { if (active) {
                    Throwable cause = e.getCause() == null ? e : e.getCause();
                    String kind = cause instanceof javax.net.ssl.SSLHandshakeException ? "tls_handshake" :
                            cause instanceof java.net.SocketTimeoutException ? "socket_timeout" : "connection_io";
                    for (Throwable nested = cause; nested != null; nested = nested.getCause())
                        if (nested instanceof android.system.ErrnoException errno) { kind = "errno_" + errno.errno; break; }
                    AppState.proxyFailure = (e instanceof StageFailure ? ((StageFailure)e).stage : "socks") + "/" + kind;
                    AppState.log("代理", "连接未完成：" + AppState.proxyFailure + "；不会绕过证书校验");
                } }
                finally { closeSocket(socket); slots.release(); }
            }); } catch (java.util.concurrent.RejectedExecutionException overloaded) { closeSocket(socket); slots.release(); }
        } catch (Exception e) { if (active) AppState.log("代理", "本地连接失败"); }
    }
    private void handle(Socket client) throws Exception {
        InputStream input = client.getInputStream(); OutputStream output = client.getOutputStream();
        byte[] greeting = TlsHello.exact(input, 2);
        if (greeting[0] != 5 || (greeting[1] & 255) == 0) return;
        byte[] methods = TlsHello.exact(input, greeting[1] & 255); boolean authentication = false;
        for (byte method : methods) if (method == 2) authentication = true;
        output.write(new byte[]{5, (byte)(authentication ? 2 : 255)}); output.flush();
        if (!authentication) return;
        byte[] auth = TlsHello.exact(input, 2);
        if (auth[0] != 1) return;
        String user = new String(TlsHello.exact(input, auth[1] & 255), StandardCharsets.UTF_8);
        int passwordSize = input.read(); if (passwordSize < 0) return;
        byte[] pass = TlsHello.exact(input, passwordSize);
        boolean ok = user.equals("farm") && MessageDigest.isEqual(pass, password.getBytes(StandardCharsets.UTF_8));
        output.write(new byte[]{1, (byte)(ok ? 0 : 1)}); output.flush(); if (!ok) return;
        AppState.socksAuthenticated++;
        byte[] request = TlsHello.exact(input, 4);
        if (request[0] != 5 || request[2] != 0) return;
        InetAddress destination;
        switch (request[3]) {
            case 1: destination = InetAddress.getByAddress(TlsHello.exact(input, 4)); break;
            case 4: destination = InetAddress.getByAddress(TlsHello.exact(input, 16)); break;
            case 3:
                int nameSize = input.read(); if (nameSize < 0) return;
                String host = new String(TlsHello.exact(input, nameSize), StandardCharsets.US_ASCII);
                destination = InetAddress.getByName(host); break;
            default: return;
        }
        byte[] portBytes = TlsHello.exact(input, 2);
        int port = ((portBytes[0] & 255) << 8) | (portBytes[1] & 255);
        if (request[1] == 3) { udp.serve(client, input, output); return; }
        if (request[1] != 1 || destination.isAnyLocalAddress() || destination.isMulticastAddress() || port == 0
                || destination.isLoopbackAddress() && port == port()) {
            AppState.socksRejected++;
            output.write(new byte[]{5, 7, 0, 1, 0, 0, 0, 0, 0, 0}); output.flush(); return;
        }
        Socket upstream;
        try { upstream = upstream(destination, port); }
        catch (IOException error) {
            // RFC 1928: never acknowledge CONNECT until the real connection exists.
            // In particular, IPv6 failure must reach the native TCP stack promptly.
            output.write(new byte[]{5, (byte)SocksReplies.failure(error), 0, 1, 0, 0, 0, 0, 0, 0}); output.flush();
            throw error;
        }
        try {
        byte[] bound = upstream.getLocalAddress().getAddress();
        int boundPort = upstream.getLocalPort();
        output.write(new byte[]{5, 0, 0, (byte)(bound.length == 16 ? 4 : 1)});
        output.write(bound); output.write(new byte[]{(byte)(boundPort >>> 8), (byte)boundPort}); output.flush();
        if (passthrough || port != 443) { opaque(client, upstream, new byte[0], passthrough); return; }
        TlsHello hello;
        try { hello = TlsHello.read(input); } catch (Exception error) { throw new StageFailure("client_hello", error); }
        AppState.observeTls(hello.serverName());
        if (!UrlPolicy.FARM_HOST.equalsIgnoreCase(hello.serverName())) {
            opaque(client, upstream, hello.consumed(), false); return;
        }
        AppState.gatewayConnections++;
        AppState.gatewayTlsAttempts++;
        try (TlsServerBridge bridge = TlsServerBridge.open(tls, client, hello.consumed(), workers)) {
            SSLSocket decrypted = bridge.socket;
            try { decrypted.startHandshake(); } catch (Exception error) { throw new StageFailure("client_tls", error); }
            AppState.tlsVerified = true;
            AppState.log("证书", "QQ 目标连接的 TLS 握手已验证成功");
            byte[] requestHeaders;
            try { requestHeaders = readHeaders(decrypted.getInputStream()); } catch (Exception error) { throw new StageFailure("client_http", error); }
            String text = new String(requestHeaders, StandardCharsets.ISO_8859_1);
            String[] first = text.split("\r\n", 2)[0].split(" ");
            String code = null;
            try {
                if (first.length == 3 && first[0].equals("GET") && header(text, "upgrade").equalsIgnoreCase("websocket"))
                    code = UrlPolicy.captureCode(first[1], hostName(header(text, "host")));
            } catch (IllegalArgumentException ignored) { /* Non-login farm traffic is forwarded without parsing. */ }
            boolean inspect = code != null && claimed.compareAndSet(false, true);
            if (inspect && !captureFriends) {
                decrypted.getOutputStream().write("HTTP/1.1 451 Unavailable For Legal Reasons\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".getBytes(StandardCharsets.US_ASCII));
                decrypted.getOutputStream().flush();
                result.accept(new AppState.Capture(code, "", Collections.emptyList(), android.os.SystemClock.elapsedRealtime(), false));
                return;
            }
            relayTls(decrypted, upstream, requestHeaders, inspect ? code : null);
        }
        } finally { closeSocket(upstream); }
    }
    private Socket upstream(InetAddress address, int port) throws IOException {
        Socket socket = new Socket();
        // A new Socket is lazy on current Android providers: it may not have a
        // native descriptor yet. Bind locally before protect(), always before
        // connect(), so protection succeeds without first leaking into the VPN.
        try { socket.bind(new InetSocketAddress(InetAddress.getByAddress(new byte[address.getAddress().length]), 0)); }
        catch (IOException error) { socket.close(); throw new StageFailure("upstream_bind", error); }
        if (!vpn.protect(socket)) { socket.close(); throw new StageFailure("upstream_protect", new IOException("无法保护上游套接字")); }
        try {
            android.net.Network selected = network.get();
            if (selected == null) throw new java.net.NoRouteToHostException("No underlying network");
            selected.bindSocket(socket);
        } catch (IOException error) { socket.close(); throw new StageFailure("upstream_network", error); }
        sockets.add(socket);
        try { socket.connect(new InetSocketAddress(address, port), 10_000); socket.setTcpNoDelay(true); socket.setSoTimeout(25_000); return socket; }
        catch (IOException e) { closeSocket(socket); throw new StageFailure(address.getAddress().length == 16 ? "upstream_connect_ipv6" : "upstream_connect_ipv4", e); }
    }
    private void opaque(Socket client, Socket server, byte[] consumed, boolean observeTls) throws Exception {
        AppState.tcpPassthroughs++;
        try {
            // Transparent QQ/MSF connections can be idle. The VPN's two-minute
            // lifetime, not an arbitrary per-read timeout, bounds these relays.
            client.setSoTimeout(0); server.setSoTimeout(0);
            server.getOutputStream().write(consumed); server.getOutputStream().flush();
            TlsMetadata metadata = observeTls ? new TlsMetadata() : null;
            workers.execute(() -> copyQuietly(client, server, metadata));
            copy(server.getInputStream(), client.getOutputStream(), metadata == null ? null : (bytes, length) -> metadata.returned(length));
        } finally { closeSocket(server); }
    }
    private void relayTls(SSLSocket client, Socket raw, byte[] request, String code) throws Exception {
        WsInspector inspector = new WsInspector(); AtomicBoolean delivered = new AtomicBoolean();
        Runnable finish = () -> {
            if (code == null || !active || !delivered.compareAndSet(false, true)) return;
            synchronized (inspector) {
                if (!inspector.friendsObserved) AppState.log("抓取", "Code 已取得，但未观察到成功的好友回包；不会伪报好友同步成功");
                result.accept(new AppState.Capture(code, inspector.ownGid, inspector.gids(), android.os.SystemClock.elapsedRealtime(), inspector.friendsObserved));
            }
        };
        try (SSLSocket server = (SSLSocket) ((SSLSocketFactory)SSLSocketFactory.getDefault()).createSocket(raw, UrlPolicy.FARM_HOST, 443, true)) {
            SSLParameters parameters = server.getSSLParameters();
            parameters.setEndpointIdentificationAlgorithm("HTTPS");
            parameters.setServerNames(Collections.singletonList(new SNIHostName(UrlPolicy.FARM_HOST)));
            server.setSSLParameters(parameters);
            try { server.startHandshake(); } catch (Exception error) { throw new StageFailure("upstream_tls", error); }
            String clean = new String(request, StandardCharsets.ISO_8859_1).replaceAll("(?im)^Sec-WebSocket-Extensions:[^\r\n]*\r\n", "");
            server.getOutputStream().write(clean.getBytes(StandardCharsets.ISO_8859_1)); server.getOutputStream().flush();
            byte[] response;
            try { response = readHeaders(server.getInputStream()); } catch (Exception error) { throw new StageFailure("upstream_http", error); }
            client.getOutputStream().write(response); client.getOutputStream().flush();
            if (code != null) {
                if (!new String(response, StandardCharsets.ISO_8859_1).startsWith("HTTP/1.1 101 ")) { finish.run(); return; }
                AppState.log("抓取", "Code 已取得，等待官方登录和好友回包（最多 20 秒）");
                deadlines.schedule(finish, 20, TimeUnit.SECONDS);
            }
            client.setSoTimeout(25_000); server.setSoTimeout(25_000);
            workers.execute(() -> copyQuietly(client, server));
            copy(server.getInputStream(), client.getOutputStream(), code == null ? null : (bytes, length) -> {
                synchronized (inspector) {
                    inspector.feed(bytes, 0, length);
                    if (inspector.friendsObserved && !inspector.ownGid.isEmpty()) finish.run();
                }
            });
        } finally { finish.run(); closeSocket(raw); }
    }
    private interface Observer { void accept(byte[] bytes, int length); }
    private void copy(InputStream input, OutputStream output, Observer observer) throws IOException {
        byte[] bytes = new byte[16384]; int length;
        while (active && (length = input.read(bytes)) != -1) {
            output.write(bytes, 0, length); output.flush();
            if (observer != null) observer.accept(bytes, length);
        }
    }
    private void copyQuietly(Socket source, Socket destination) {
        copyQuietly(source, destination, null);
    }
    private void copyQuietly(Socket source, Socket destination, TlsMetadata metadata) {
        try { copy(source.getInputStream(), destination.getOutputStream(), metadata == null ? null : metadata::feed); }
        catch (IOException ignored) { }
        finally { try { destination.shutdownOutput(); } catch (IOException ignored) { } }
    }
    static byte[] readHeaders(InputStream input) throws IOException {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream(); int state = 0;
        while (bytes.size() < 32768) {
            int b = input.read(); if (b == -1) throw new IOException("HTTP 头未完整接收"); bytes.write(b);
            state = (state == 0 && b == 13) ? 1 : (state == 1 && b == 10) ? 2 : (state == 2 && b == 13) ? 3 : (state == 3 && b == 10) ? 4 : (b == 13 ? 1 : 0);
            if (state == 4) return bytes.toByteArray();
        }
        throw new IOException("HTTP 头超过安全限制");
    }
    private static String header(String text, String name) {
        String result = "";
        for (String line : text.split("\r\n")) {
            int colon = line.indexOf(':');
            if (colon > 0 && line.substring(0, colon).equalsIgnoreCase(name)) {
                if (!result.isEmpty()) return "";
                result = line.substring(colon + 1).trim();
            }
        }
        return result;
    }
    private static String hostName(String host) { return host.endsWith(":443") ? host.substring(0, host.length() - 4) : host; }
    private void closeSocket(Socket socket) { sockets.remove(socket); try { socket.close(); } catch (IOException ignored) { } }
    @Override public void close() {
        active = false;
        try { listener.close(); } catch (IOException ignored) { }
        for (Socket socket : new ArrayList<>(sockets)) closeSocket(socket);
        udp.close();
        deadlines.shutdownNow(); workers.shutdownNow();
    }
}
