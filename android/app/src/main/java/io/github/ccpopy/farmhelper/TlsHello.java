package io.github.ccpopy.farmhelper;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;

/** Inspect only plaintext SNI, preserving bytes for either TLS or opaque passthrough. */
public record TlsHello(byte[] consumed, String serverName) {
    public static TlsHello read(InputStream input) throws IOException {
        ByteArrayOutputStream consumed = new ByteArrayOutputStream();
        ByteArrayOutputStream handshake = new ByteArrayOutputStream();
        while (consumed.size() < 65536) {
            byte[] header = exact(input, 5); consumed.write(header);
            int length = ((header[3] & 255) << 8) | (header[4] & 255);
            if (length > 18432 || consumed.size() + length > 65536) throw new IOException("TLS 握手超过安全限制");
            byte[] record = exact(input, length); consumed.write(record);
            if (header[0] != 22 || header[1] != 3) return new TlsHello(consumed.toByteArray(), "");
            handshake.write(record);
            byte[] body = handshake.toByteArray();
            if (body.length >= 4) {
                if (body[0] != 1) return new TlsHello(consumed.toByteArray(), "");
                int total = ((body[1] & 255) << 16) | ((body[2] & 255) << 8) | (body[3] & 255);
                if (total > 65532) throw new IOException("TLS ClientHello 过大");
                if (body.length >= total + 4) return new TlsHello(consumed.toByteArray(), name(body));
            }
        }
        throw new IOException("TLS 握手过大");
    }
    private static String name(byte[] data) throws IOException {
        try {
            int p = 4 + 2 + 32;
            p += 1 + u8(data, p); p += 2 + u16(data, p); p += 1 + u8(data, p);
            if (p == data.length) return "";
            int end = p + 2 + u16(data, p); p += 2;
            if (end > data.length) throw new IOException("TLS 扩展损坏");
            while (p + 4 <= end) {
                int type = u16(data, p), length = u16(data, p + 2); p += 4;
                if (p + length > end) throw new IOException("TLS 扩展损坏");
                if (type == 0) {
                    int listEnd = p + 2 + u16(data, p), q = p + 2;
                    if (listEnd > p + length) throw new IOException("SNI 损坏");
                    while (q + 3 <= listEnd) {
                        int kind = u8(data, q), size = u16(data, q + 1); q += 3;
                        if (q + size > listEnd) throw new IOException("SNI 损坏");
                        if (kind == 0) return new String(data, q, size, StandardCharsets.US_ASCII);
                        q += size;
                    }
                }
                p += length;
            }
            return "";
        } catch (IndexOutOfBoundsException e) { throw new IOException("TLS 握手损坏"); }
    }
    private static int u8(byte[] data, int p) { return data[p] & 255; }
    private static int u16(byte[] data, int p) { return u8(data, p) * 256 + u8(data, p + 1); }
    static byte[] exact(InputStream input, int count) throws IOException {
        byte[] bytes = new byte[count]; int p = 0;
        while (p < count) { int n = input.read(bytes, p, count - p); if (n == -1) throw new IOException("连接已关闭"); p += n; }
        return bytes;
    }
}
