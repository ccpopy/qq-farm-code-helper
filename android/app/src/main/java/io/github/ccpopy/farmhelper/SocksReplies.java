package io.github.ccpopy.farmhelper;

/** RFC 1928 failure codes. No endpoint or exception messages are exposed to logs. */
final class SocksReplies {
    static int failure(Throwable error) {
        for (Throwable cause = error; cause != null; cause = cause.getCause()) {
            if (cause instanceof java.net.NoRouteToHostException) return 3;
            if (cause instanceof java.net.SocketTimeoutException) return 6;
            if (cause instanceof java.net.ConnectException) return 5;
            if (cause instanceof java.net.UnknownHostException) return 4;
        }
        return 1;
    }
    private SocksReplies() {}
}
