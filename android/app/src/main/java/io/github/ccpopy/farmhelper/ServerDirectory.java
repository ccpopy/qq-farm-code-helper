package io.github.ccpopy.farmhelper;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletionService;
import java.util.concurrent.ExecutorCompletionService;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.function.Consumer;

/** Reads every saved server at once, so a slow or offline server never hides the others. */
final class ServerDirectory {
    private static final ExecutorService LOAD = Executors.newCachedThreadPool();
    private ServerDirectory() {}

    /** One server's accounts, or the reason they could not be read. */
    record Group(SettingsStore server, ApiClient client, List<ApiClient.Account> accounts, String error, String connectionId) {
        Group(SettingsStore server, ApiClient client, List<ApiClient.Account> accounts, String error) {
            this(server, client, accounts, error, client == null ? server.connectionId() : client.connectionId);
        }
        boolean ok() { return error == null; }
        boolean current() { return server.ready() && connectionId.equals(server.connectionId()); }
    }
    record Choice(Group group, ApiClient.Account account) {}

    /** Load all ready servers in parallel; {@code platform} filters accounts when non-null. */
    static List<Group> load(CapturePlatform platform) {
        return load(platform, group -> {});
    }
    /** Deliver each completed server immediately on the calling worker; return groups in saved order. */
    static List<Group> load(CapturePlatform platform, Consumer<Group> completed) {
        List<SettingsStore> servers = AppState.settings.readyServers();
        CompletionService<Group> results = new ExecutorCompletionService<>(LOAD);
        Map<Future<Group>, Integer> pending = new HashMap<>();
        List<Group> groups = new ArrayList<>(Collections.nCopies(servers.size(), null));
        for (int i = 0; i < servers.size(); i++) {
            SettingsStore server = servers.get(i);
            pending.put(results.submit(() -> read(server, platform)), i);
        }
        try {
            for (int count = 0; count < servers.size(); count++) {
                Future<Group> next = results.take();
                int index = pending.remove(next);
                Group group;
                try { group = next.get(); }
                catch (java.util.concurrent.ExecutionException failed) {
                    group = new Group(servers.get(index), null, List.of(), "读取失败");
                }
                groups.set(index, group); completed.accept(group);
            }
        } catch (InterruptedException interrupted) {
            for (Future<Group> future : pending.keySet()) future.cancel(true);
            Thread.currentThread().interrupt();
            for (int i = 0; i < groups.size(); i++) if (groups.get(i) == null) {
                Group group = new Group(servers.get(i), null, List.of(), "读取中断");
                groups.set(i, group); completed.accept(group);
            }
        }
        return groups;
    }
    static Group read(SettingsStore server, CapturePlatform platform) {
        String connection = server.connectionId();
        ApiClient client = null;
        try {
            client = new ApiClient(server);
            List<ApiClient.Account> accounts = new ArrayList<>(client.accounts());
            if (platform != null) accounts.removeIf(account -> !account.platform.equals(platform.id));
            return new Group(server, client, accounts, null, connection);
        } catch (Exception error) {
            return new Group(server, client, List.of(), reason(error), connection);
        }
    }
    static String reason(Exception error) {
        if (error instanceof ApiClient.AuthException) return "Token 无效";
        if (error instanceof ApiClient.TemporaryNetworkException || error instanceof java.net.ConnectException
                || error instanceof java.net.UnknownHostException) return "无法连接";
        String text = error.getMessage();
        return text == null || text.length() > 60 || text.contains("://") ? "读取失败" : text;
    }

    /** The account whose game identity matches the capture, only if exactly one exists across servers. */
    static Choice detect(List<Group> groups, AppState.Capture capture) {
        if (capture.ownGid().isEmpty()) return null;
        Choice found = null;
        for (Group group : groups) {
            if (!group.ok() || !group.current()) continue;
            for (ApiClient.Account account : group.accounts()) {
                if (!account.platform.equals(capture.platform().id) || !capture.ownGid().equals(group.client().knownGid(account))) continue;
                if (found != null) return null;
                found = new Choice(group, account);
            }
        }
        return found;
    }
    /** Where the user pushed last time, unless the captured identity contradicts it. */
    static Choice previous(List<Group> groups, AppState.Capture capture) {
        String last = AppState.settings.lastServer();
        for (Group group : groups) {
            if (!group.ok() || !group.current() || !group.server().id().equals(last)) continue;
            String target = group.server().targetId();
            for (ApiClient.Account account : group.accounts()) {
                if (!account.id.equals(target)) continue;
                String known = group.client().knownGid(account), own = capture.ownGid();
                if (own.isEmpty() || known.isEmpty() || own.equals(known)) return new Choice(group, account);
            }
        }
        return null;
    }
    static boolean allCurrent(List<Group> groups) {
        for (Group group : groups) if (group.ok() && !group.current()) return false;
        return true;
    }
    static int accountCount(List<Group> groups) {
        int count = 0;
        for (Group group : groups) count += group.accounts().size();
        return count;
    }
}
