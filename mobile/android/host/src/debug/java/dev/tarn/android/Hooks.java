package dev.tarn.android;

import android.content.Context;
import android.util.Log;
import android.webkit.WebView;

import androidx.webkit.ProxyConfig;
import androidx.webkit.ProxyController;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import androidx.webkit.WebViewRenderProcess;

import java.io.IOException;
import java.net.InetSocketAddress;
import java.net.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Pattern;

final class Hooks {
    private static final String TAG = "tarn";
    private static final Pattern PROXY = Pattern.compile("^(localhost|127\\.0\\.0\\.1):[0-9]{1,5}$");

    static final boolean LOGS = true;

    private Hooks() {}

    static void log(String message) {
        Log.i(TAG, message);
    }

    private static String debugProxy(Context context) {
        if (context == null) return null;
        try {
            String proxy = new String(Files.readAllBytes(context.getDataDir().toPath().resolve("debug-proxy")), StandardCharsets.UTF_8).trim();
            if (!PROXY.matcher(proxy).matches()) return null;
            int port = Integer.parseInt(proxy.substring(proxy.lastIndexOf(':') + 1));
            return port >= 1 && port <= 65535 ? proxy : null;
        } catch (IOException e) {
            return null;
        }
    }

    static Proxy proxy() {
        String proxy = debugProxy(Host.app);
        if (proxy == null) return Proxy.NO_PROXY;
        int colon = proxy.lastIndexOf(':');
        return new Proxy(Proxy.Type.HTTP, new InetSocketAddress(proxy.substring(0, colon), Integer.parseInt(proxy.substring(colon + 1))));
    }

    static void configure(Context context) {
        String proxy = debugProxy(context);
        if (proxy == null || !WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE)) {
            Log.w(TAG, "debug proxy ignored");
            return;
        }
        ProxyConfig config = new ProxyConfig.Builder().addProxyRule(proxy).build();
        ProxyController.getInstance().setProxyOverride(config, Runnable::run, () -> log("debug proxy " + proxy));
    }

    static boolean command(String name, WebView web) {
        if (!"debug_crash".equals(name) || web == null
                || !WebViewFeature.isFeatureSupported(WebViewFeature.WEB_VIEW_RENDERER_TERMINATE)) {
            return false;
        }
        WebViewRenderProcess process = WebViewCompat.getWebViewRenderProcess(web);
        log("debug renderer terminated=" + (process != null && process.terminate()));
        return true;
    }
}
