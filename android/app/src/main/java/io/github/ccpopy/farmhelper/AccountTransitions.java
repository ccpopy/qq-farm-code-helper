package io.github.ccpopy.farmhelper;

import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;

/** A network error never becomes an offline event. Two observations debounce reconnects. */
public final class AccountTransitions {
    private final Map<String, Integer> offlineCount = new HashMap<>();
    private final Set<String> notified = new HashSet<>();
    public boolean observe(String id, boolean online) {
        if (online) { offlineCount.remove(id); notified.remove(id); return false; }
        int count = offlineCount.merge(id, 1, (old, increment) -> Math.min(2, old + increment));
        // Also alert for accounts already offline when monitoring starts (after confirmation).
        if (count < 2 || notified.contains(id)) return false;
        notified.add(id);
        return true;
    }
    /** Failed checks break consecutiveness, but must not re-alert an already notified account. */
    public void interrupted() { offlineCount.clear(); }
    public void retain(Set<String> ids) {
        offlineCount.keySet().retainAll(ids); notified.retainAll(ids);
    }
}
