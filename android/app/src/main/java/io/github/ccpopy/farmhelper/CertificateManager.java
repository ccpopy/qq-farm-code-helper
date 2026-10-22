package io.github.ccpopy.farmhelper;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import java.nio.file.Files;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.security.PrivateKey;
import java.security.cert.Certificate;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.util.Arrays;
import java.util.Collections;
import java.util.Date;
import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.security.auth.x500.X500Principal;

public final class CertificateManager {
    public static final String NAME = "Farm Helper Local CA";
    private static final String ALIAS = "farmhelper.local-ca.rsa.v1";
    private final java.io.File file;
    private final String alias;
    public CertificateManager(Context context) { this(context, "local-ca.cer", ALIAS); }
    CertificateManager(Context context, String filename, String alias) {
        file = new java.io.File(context.getFilesDir(), filename); this.alias = alias;
    }
    public synchronized boolean exists() { return file.isFile(); }
    public synchronized X509Certificate certificate() throws Exception {
        try (var input = Files.newInputStream(file.toPath())) {
            return (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(input);
        }
    }
    public synchronized byte[] export() throws Exception { return certificate().getEncoded(); }
    public synchronized String fingerprint() throws Exception {
        byte[] hash = MessageDigest.getInstance("SHA-256").digest(export());
        StringBuilder value = new StringBuilder();
        for (byte b : hash) { if (value.length() > 0) value.append(':'); value.append(String.format(java.util.Locale.ROOT, "%02X", b)); }
        return value.toString();
    }
    public synchronized boolean visibleInSystem() throws Exception {
        if (!exists()) return false;
        byte[] encoded = export();
        KeyStore store = KeyStore.getInstance("AndroidCAStore"); store.load(null);
        for (String alias : Collections.list(store.aliases())) {
            Certificate certificate = store.getCertificate(alias);
            if (certificate != null && Arrays.equals(encoded, certificate.getEncoded())) return true;
        }
        return false;
    }
    public synchronized void generate() throws Exception {
        if (exists()) { certificate().checkValidity(); return; }
        KeyStore store = keyStore();
        if (!store.containsAlias(alias)) {
            KeyPairGenerator generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, "AndroidKeyStore");
            generator.initialize(new KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN | KeyProperties.PURPOSE_VERIFY)
                    .setKeySize(2048).setDigests(KeyProperties.DIGEST_SHA256)
                    .setSignaturePaddings(KeyProperties.SIGNATURE_PADDING_RSA_PKCS1)
                    .setCertificateSubject(new X500Principal("CN=" + NAME)).build());
            generator.generateKeyPair();
        }
        PrivateKey key = (PrivateKey) store.getKey(alias, null);
        X509Certificate ca = DerCertificate.issue(NAME, store.getCertificate(alias).getPublicKey(), NAME, key, true,
                new Date(System.currentTimeMillis() - 300_000), new Date(System.currentTimeMillis() + 365L * 86_400_000));
        Files.write(file.toPath(), ca.getEncoded());
    }
    public synchronized SSLContext serverContext() throws Exception {
        if (!exists()) throw new IllegalStateException("请先在权限与证书页面生成并安装证书");
        X509Certificate root = certificate(); root.checkValidity();
        KeyStore android = keyStore();
        PrivateKey caKey = (PrivateKey) android.getKey(alias, null);
        if (caKey == null) throw new IllegalStateException("本地证书私钥不存在，请移除旧证书并重新生成");
        KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA"); generator.initialize(2048);
        KeyPair pair = generator.generateKeyPair();
        X509Certificate leaf = DerCertificate.issue(UrlPolicy.FARM_HOST, pair.getPublic(), NAME, caKey, false,
                new Date(System.currentTimeMillis() - 300_000), new Date(Math.min(root.getNotAfter().getTime(), System.currentTimeMillis() + 7L * 86_400_000)));
        leaf.verify(root.getPublicKey());
        KeyStore keys = KeyStore.getInstance("PKCS12"); keys.load(null);
        char[] password = new char[0];
        keys.setKeyEntry("leaf", pair.getPrivate(), password, new Certificate[]{leaf, root});
        KeyManagerFactory km = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()); km.init(keys, password);
        SSLContext context = SSLContext.getInstance("TLS"); context.init(km.getKeyManagers(), null, null);
        return context;
    }
    /** This only deletes this app's material, NOT the installed system/user CA copy. */
    public synchronized void deleteLocal() throws Exception {
        if (AppState.capturing) throw new IllegalStateException("请先停止 VPN 再移除证书");
        keyStore().deleteEntry(alias);
        Files.deleteIfExists(file.toPath());
        AppState.tlsVerified = false;
    }
    private static KeyStore keyStore() throws Exception { KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null); return store; }
}
