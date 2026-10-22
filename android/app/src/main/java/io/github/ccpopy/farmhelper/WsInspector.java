package io.github.ccpopy.farmhelper;

import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashSet;
import java.util.List;

/** Passive RFC 6455 / protobuf reader. Never retains names, avatars, OpenIDs or full frames. */
public final class WsInspector {
    private static final int MAX = 8 * 1024 * 1024;
    private byte[] buffer = new byte[8192];
    private int start, end;
    private boolean valid = true;
    private ByteArrayOutputStream fragmented;
    private int fragmentType;
    public String ownGid = "";
    public boolean friendsObserved;
    private final LinkedHashSet<String> gids = new LinkedHashSet<>();
    public boolean valid() { return valid; }
    public List<String> gids() { List<String> result = new ArrayList<>(gids); result.remove(ownGid); return result; }
    public void feed(byte[] data, int offset, int length) {
        if (!valid || length == 0) return;
        if (length > MAX + 14 - (end - start)) { disable(); return; }
        if (buffer.length - end < length) {
            int size = end - start;
            byte[] next = new byte[Math.min(MAX + 14, Math.max(size + length, buffer.length * 2))];
            System.arraycopy(buffer, start, next, 0, size); buffer = next; start = 0; end = size;
        }
        System.arraycopy(data, offset, buffer, end, length); end += length;
        while (end - start >= 2) {
            int first = buffer[start] & 255, second = buffer[start + 1] & 255;
            int opcode = first & 15; boolean fin = (first & 128) != 0;
            if ((first & 112) != 0 || (second & 128) != 0 || !(opcode == 0 || opcode == 1 || opcode == 2 || opcode == 8 || opcode == 9 || opcode == 10)) { disable(); return; }
            int header = 2; long payload = second & 127;
            if (payload == 126) {
                if (end - start < 4) return;
                payload = ((buffer[start + 2] & 255) << 8) | (buffer[start + 3] & 255); header = 4;
                if (payload < 126) { disable(); return; }
            } else if (payload == 127) {
                if (end - start < 10) return;
                payload = 0;
                for (int i = 2; i < 10; i++) { if (payload > MAX) { disable(); return; } payload = payload * 256 + (buffer[start + i] & 255); }
                header = 10;
                if (payload < 65536) { disable(); return; }
            }
            if (payload > MAX || opcode >= 8 && (!fin || payload > 125)) { disable(); return; }
            if (end - start < header + payload) return;
            int pos = start + header, count = (int)payload;
            if (opcode <= 2) {
                if (opcode != 0) {
                    if (fragmentType != 0) { disable(); return; }
                    if (fin) { if (opcode == 2) inspect(Arrays.copyOfRange(buffer, pos, pos + count)); }
                    else { fragmentType = opcode; fragmented = new ByteArrayOutputStream(); if (opcode == 2) fragmented.write(buffer, pos, count); }
                } else {
                    if (fragmentType == 0) { disable(); return; }
                    if (fragmentType == 2) {
                        if (fragmented.size() + count > MAX) { disable(); return; }
                        fragmented.write(buffer, pos, count);
                    }
                    if (fin) { if (fragmentType == 2) inspect(fragmented.toByteArray()); fragmented = null; fragmentType = 0; }
                }
            }
            start += header + count;
            if (start == end) { start = end = 0; break; }
        }
    }
    private void disable() { valid = false; buffer = new byte[0]; start = end = 0; fragmented = null; }
    private void inspect(byte[] message) {
        try {
            Proto envelope = new Proto(message); byte[] meta = null, body = null;
            while (envelope.next()) {
                if (envelope.field == 1 && envelope.wire == 2) meta = envelope.bytes();
                else if (envelope.field == 2 && envelope.wire == 2) body = envelope.bytes();
                else envelope.skip();
            }
            if (meta == null || body == null) return;
            Proto m = new Proto(meta); String service = "", method = ""; long type = 0, error = 0;
            while (m.next()) {
                if (m.field == 1 && m.wire == 2) service = new String(m.bytes(), StandardCharsets.UTF_8);
                else if (m.field == 2 && m.wire == 2) method = new String(m.bytes(), StandardCharsets.UTF_8);
                else if (m.field == 3 && m.wire == 0) type = m.varint();
                else if (m.field == 6 && m.wire == 0) error = m.varint();
                else m.skip();
            }
            if (type != 2 || error != 0) return;
            if (service.equals("gamepb.userpb.UserService") && method.equals("Login")) {
                Proto login = new Proto(body);
                while (login.next()) {
                    if (login.field == 1 && login.wire == 2) { String own = firstGid(login.bytes()); if (!own.isEmpty()) ownGid = own; }
                    else login.skip();
                }
            } else if (service.equals("gamepb.friendpb.FriendService") && (method.equals("SyncAll") || method.equals("GetGameFriends"))) {
                Proto friends = new Proto(body); LinkedHashSet<String> found = new LinkedHashSet<>();
                while (friends.next()) {
                    if (friends.field == 1 && friends.wire == 2) { String gid = firstGid(friends.bytes()); if (!gid.isEmpty() && found.size() < 500) found.add(gid); }
                    else friends.skip();
                }
                friendsObserved = true; gids.clear(); gids.addAll(found);
            }
        } catch (IllegalArgumentException ignored) { /* Malformed protobuf is not a successful capture. */ }
    }
    private static String firstGid(byte[] data) {
        Proto entry = new Proto(data); String gid = "";
        while (entry.next()) {
            if (entry.field == 1 && entry.wire == 0) { long n = entry.varint(); if (n > 0 && n <= 9_007_199_254_740_991L) gid = Long.toString(n); }
            else entry.skip();
        }
        return gid;
    }
    private static final class Proto {
        final byte[] data; int position, field, wire;
        Proto(byte[] data) { this.data = data; }
        boolean next() {
            if (position == data.length) return false;
            long tag = varint(); field = (int)(tag >>> 3); wire = (int)(tag & 7);
            if (field <= 0 || tag > 0xffffffffL) throw new IllegalArgumentException();
            return true;
        }
        long varint() {
            long n = 0;
            for (int i = 0; i < 10; i++) {
                if (position >= data.length) throw new IllegalArgumentException();
                int b = data[position++] & 255;
                if (i == 9 && b > 1) throw new IllegalArgumentException();
                n |= (long)(b & 127) << (i * 7);
                if ((b & 128) == 0) return n;
            }
            throw new IllegalArgumentException();
        }
        byte[] bytes() {
            long size = varint();
            if (size < 0 || size > data.length - position) throw new IllegalArgumentException();
            byte[] result = Arrays.copyOfRange(data, position, position + (int)size); position += (int)size; return result;
        }
        void skip() {
            long n;
            switch (wire) {
                case 0: varint(); return;
                case 1: n = 8; break;
                case 2: n = varint(); break;
                case 5: n = 4; break;
                default: throw new IllegalArgumentException();
            }
            if (n < 0 || n > data.length - position) throw new IllegalArgumentException();
            position += (int)n;
        }
    }
}
