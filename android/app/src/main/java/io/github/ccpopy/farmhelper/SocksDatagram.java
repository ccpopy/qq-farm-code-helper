package io.github.ccpopy.farmhelper;

import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;

/** SOCKS5 UDP framing only. Payloads are forwarded, never interpreted or persisted. */
final class SocksDatagram {
    record Frame(InetSocketAddress target, int offset, int length) {}
    static Frame decode(byte[] bytes, int length) throws IOException {
        if (length < 4 || length > bytes.length || bytes[0] != 0 || bytes[1] != 0 || bytes[2] != 0)
            throw new IOException("Unsupported UDP framing");
        int size = bytes[3] == 1 ? 4 : bytes[3] == 4 ? 16 : 0;
        if (size == 0 || length < 6 + size) throw new IOException("Unsupported UDP address");
        byte[] ip = java.util.Arrays.copyOfRange(bytes, 4, 4 + size);
        int port = (bytes[4 + size] & 255) * 256 + (bytes[5 + size] & 255);
        InetAddress address = InetAddress.getByAddress(ip);
        if (port == 0 || address.isAnyLocalAddress() || address.isMulticastAddress()) throw new IOException("Invalid UDP endpoint");
        return new Frame(new InetSocketAddress(address, port), 6 + size, length - 6 - size);
    }
    static byte[] encode(InetSocketAddress source, byte[] payload, int length) {
        if (length < 0 || length > payload.length || length > 65507) throw new IllegalArgumentException("UDP payload size");
        byte[] ip = source.getAddress().getAddress(), packet = new byte[6 + ip.length + length];
        packet[3] = (byte)(ip.length == 4 ? 1 : 4);
        System.arraycopy(ip, 0, packet, 4, ip.length);
        packet[4 + ip.length] = (byte)(source.getPort() >>> 8); packet[5 + ip.length] = (byte)source.getPort();
        System.arraycopy(payload, 0, packet, 6 + ip.length, length);
        return packet;
    }
}
