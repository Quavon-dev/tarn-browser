package dev.tarn.android;

import android.content.Context;
import android.webkit.WebView;

import java.net.Proxy;

final class Hooks {
    static final boolean LOGS = false;

    private Hooks() {}

    static void log(String message) {}

    static void configure(Context context) {}

    static Proxy proxy() {
        return Proxy.NO_PROXY;
    }

    static boolean command(String name, WebView web) {
        return false;
    }
}
