package io.github.ccpopy.farmhelper;

/** The upstream treats an ID-less request with an existing remark as a relogin. */
public final class AccountNamePolicy {
    private AccountNamePolicy() {}
    public static String base(String input) {
        String value = input.trim();
        if (value.isEmpty() || value.length() > 60 || value.chars().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("账号备注须为 1–60 个字符，不能包含换行或控制字符");
        return value;
    }
    public static String unique(String input, String nonce) {
        if (!nonce.matches("[a-f0-9]{12}")) throw new IllegalArgumentException("新增账号标识无效");
        return base(input) + " · 手机-" + nonce;
    }
    public static void validate(String input, String actual) {
        String prefix = base(input) + " · 手机-";
        if (!actual.startsWith(prefix) || !actual.substring(prefix.length()).matches("[a-f0-9]{12}"))
            throw new IllegalArgumentException("新增账号备注与确认内容不一致");
    }
}
