package io.github.ccpopy.farmhelper;

/** Passive ClientHello metadata while bytes are already being forwarded. Never delays a relay. */
final class TlsMetadata {
    private java.io.ByteArrayOutputStream pending = new java.io.ByteArrayOutputStream();
    private String host = "";
    private boolean received, reported;
    synchronized void feed(byte[] bytes, int length) {
        if (pending == null) return;
        if (pending.size() + length > 65536) { pending = null; return; }
        pending.write(bytes, 0, length);
        byte[] hello = pending.toByteArray();
        if (hello.length < 5) return;
        if (hello[0] != 22 || hello[1] != 3) { pending = null; return; }
        try {
            TlsHello parsed = TlsHello.read(new java.io.ByteArrayInputStream(hello));
            host = parsed.serverName(); AppState.observeTls(host); pending = null; reportReply();
        } catch (java.io.IOException incomplete) { /* Retry with the next bytes, without holding up forwarding. */ }
    }
    synchronized void returned(int length) { received |= length > 0; reportReply(); }
    private void reportReply() { if (received && !reported && !host.isEmpty()) { reported = true; AppState.observeTlsReply(host); } }
}
