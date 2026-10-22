package io.github.ccpopy.farmhelper;

import java.io.Closeable;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.util.concurrent.Executor;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLServerSocket;
import javax.net.ssl.SSLSocket;

/** Public Android TLS APIs do not expose consumed-ClientHello sockets. Replay through a private loopback pair. */
public final class TlsServerBridge implements Closeable {
    public final SSLSocket socket;
    private final Socket replay;
    private final java.util.concurrent.CountDownLatch outboundDrained;
    private TlsServerBridge(SSLSocket socket, Socket replay, java.util.concurrent.CountDownLatch drained) {
        this.socket = socket; this.replay = replay; this.outboundDrained = drained;
    }
    public static TlsServerBridge open(SSLContext context, Socket client, byte[] consumed, Executor executor) throws IOException {
        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        Socket replay = new Socket(); SSLSocket accepted = null;
        try (SSLServerSocket listener = (SSLServerSocket) context.getServerSocketFactory().createServerSocket(0, 1, loopback)) {
            listener.setSoTimeout(3000);
            replay.connect(new InetSocketAddress(loopback, listener.getLocalPort()), 3000);
            replay.setTcpNoDelay(true); replay.setSoTimeout(25_000);
            accepted = (SSLSocket) listener.accept();
            // Do not accept a connection from another local app that raced our ephemeral listener.
            if (!accepted.getInetAddress().equals(loopback) || accepted.getPort() != replay.getLocalPort())
                throw new IOException("本地 TLS 回放连接身份不符");
            accepted.setUseClientMode(false); accepted.setSoTimeout(15_000);
            replay.getOutputStream().write(consumed); replay.getOutputStream().flush();
            var drained = new java.util.concurrent.CountDownLatch(1);
            executor.execute(() -> pump(client, replay));
            executor.execute(() -> { try { pump(replay, client); } finally { drained.countDown(); } });
            return new TlsServerBridge(accepted, replay, drained);
        } catch (IOException | RuntimeException error) {
            replay.close(); if (accepted != null) accepted.close(); throw error;
        }
    }
    private static void pump(Socket source, Socket destination) {
        try {
            byte[] bytes = new byte[16384]; int size;
            while ((size = source.getInputStream().read(bytes)) != -1) {
                destination.getOutputStream().write(bytes, 0, size); destination.getOutputStream().flush();
            }
        } catch (IOException ignored) { }
        finally { try { destination.shutdownOutput(); } catch (IOException ignored) { } }
    }
    @Override public void close() throws IOException {
        try { socket.close(); }
        finally {
            // TLS close queues the last encrypted records on the loopback pair. Closing
            // replay immediately races its outbound pump and can silently drop the reply.
            try { outboundDrained.await(1000, java.util.concurrent.TimeUnit.MILLISECONDS); }
            catch (InterruptedException interrupted) { Thread.currentThread().interrupt(); }
            finally { replay.close(); }
        }
    }
}
