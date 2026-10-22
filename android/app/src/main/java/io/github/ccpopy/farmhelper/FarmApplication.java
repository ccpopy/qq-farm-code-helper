package io.github.ccpopy.farmhelper;

import android.app.Application;

public final class FarmApplication extends Application {
    @Override public void onCreate() {
        super.onCreate();
        AppState.init(this);
        Notifications.createChannels(this);
    }
}
