package io.github.ccpopy.farmhelper;

/** QQ numbers and game GIDs are separate namespaces, even when their digits happen to match. */
final class AccountIdentity {
    private AccountIdentity() {}
    static String qq(String value) {
        String number = value == null ? "" : value.trim();
        return number.matches("[1-9][0-9]{4,11}") ? number : "";
    }
    static String gid(String value) {
        String number = value == null ? "" : value.trim();
        if (!number.matches("[1-9][0-9]{0,15}")) return "";
        return Long.parseLong(number) <= 9_007_199_254_740_991L ? number : "";
    }
}
