package io.github.ccpopy.farmhelper;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.PrivateKey;
import java.security.PublicKey;
import java.security.SecureRandom;
import java.security.Signature;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.time.Instant;
import java.time.ZoneOffset;
import java.time.format.DateTimeFormatter;
import java.util.Date;

/** Narrow DER encoder for our RSA/SHA-256 root and one DNS-only leaf; no crypto provider dependency. */
public final class DerCertificate {
    private static final byte[] RSA_SHA256 = seq(oid("1.2.840.113549.1.1.11"), tag(5, new byte[0]));
    private DerCertificate() {}
    public static X509Certificate issue(String subject, PublicKey publicKey, String issuer, PrivateKey issuerKey,
                                         boolean ca, Date notBefore, Date notAfter) throws Exception {
        byte[] extensions;
        if (ca) {
            extensions = seq(extension("2.5.29.19", true, seq(tag(1, new byte[]{(byte)255}), integer(BigInteger.ZERO))),
                    extension("2.5.29.15", true, tag(3, new byte[]{1, 6})));
        } else {
            extensions = seq(extension("2.5.29.19", true, seq()),
                    extension("2.5.29.15", true, tag(3, new byte[]{5, (byte)0xa0})),
                    extension("2.5.29.37", false, seq(oid("1.3.6.1.5.5.7.3.1"))),
                    extension("2.5.29.17", false, seq(tag(0x82, subject.getBytes(StandardCharsets.US_ASCII)))));
        }
        byte[] tbs = seq(tag(0xa0, integer(BigInteger.valueOf(2))),
                integer(new BigInteger(128, new SecureRandom()).setBit(127)), RSA_SHA256, name(issuer),
                seq(time(notBefore), time(notAfter)), name(subject), publicKey.getEncoded(), tag(0xa3, extensions));
        Signature signature = Signature.getInstance("SHA256withRSA");
        signature.initSign(issuerKey); signature.update(tbs);
        byte[] signed = signature.sign();
        byte[] bitString = new byte[signed.length + 1];
        System.arraycopy(signed, 0, bitString, 1, signed.length);
        return (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(new ByteArrayInputStream(seq(tbs, RSA_SHA256, tag(3, bitString))));
    }
    private static byte[] name(String value) { return seq(tag(0x31, seq(oid("2.5.4.3"), tag(0x0c, value.getBytes(StandardCharsets.UTF_8))))); }
    private static byte[] time(Date value) {
        return tag(0x17, DateTimeFormatter.ofPattern("yyMMddHHmmss'Z'").withZone(ZoneOffset.UTC)
                .format(Instant.ofEpochMilli(value.getTime())).getBytes(StandardCharsets.US_ASCII));
    }
    private static byte[] extension(String oid, boolean critical, byte[] content) {
        return critical ? seq(oid(oid), tag(1, new byte[]{(byte)255}), tag(4, content)) : seq(oid(oid), tag(4, content));
    }
    private static byte[] integer(BigInteger n) { return tag(2, n.toByteArray()); }
    private static byte[] oid(String value) {
        String[] parts = value.split("\\.");
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(Integer.parseInt(parts[0]) * 40 + Integer.parseInt(parts[1]));
        for (int i = 2; i < parts.length; i++) {
            long n = Long.parseLong(parts[i]); byte[] encoded = new byte[10]; int index = encoded.length;
            encoded[--index] = (byte)(n & 127);
            while ((n >>= 7) > 0) encoded[--index] = (byte)((n & 127) | 128);
            out.write(encoded, index, encoded.length - index);
        }
        return tag(6, out.toByteArray());
    }
    private static byte[] seq(byte[]... parts) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (byte[] bytes : parts) out.write(bytes, 0, bytes.length);
        return tag(0x30, out.toByteArray());
    }
    private static byte[] tag(int type, byte[] body) {
        ByteArrayOutputStream out = new ByteArrayOutputStream(); out.write(type);
        if (body.length < 128) out.write(body.length);
        else {
            int count = 0, length = body.length;
            while (length > 0) { count++; length >>= 8; }
            out.write(128 | count);
            for (int i = count - 1; i >= 0; i--) out.write(body.length >> (8 * i));
        }
        out.write(body, 0, body.length);
        return out.toByteArray();
    }
}
