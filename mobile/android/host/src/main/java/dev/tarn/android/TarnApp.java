package dev.tarn.android;

import android.app.Application;
import android.util.Log;
import android.webkit.ServiceWorkerClient;
import android.webkit.ServiceWorkerController;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;

import java.io.ByteArrayInputStream;
import java.util.Collections;

public final class TarnApp extends Application {
    private static final String TAG = "tarn";

    @Override
    public void onCreate() {
        super.onCreate();
        Host.app = this;
        if (getProcessName().endsWith(":private")) return;
        if (!PrivateSpace.running(this)) PrivateSpace.sweep(this, true);
        if (!Native.start(Host.class, getFilesDir().getPath(), null)) Log.e(TAG, "native host not started");
        warm();
        interceptServiceWorkers();
        Hooks.configure(this);
        Stores.cleanUpAtLaunch(this);
        Downloads.sweep(this);
    }

    static final ServiceWorkerClient SERVICE_WORKERS = new ServiceWorkerClient() {
        @Override
        public WebResourceResponse shouldInterceptRequest(WebResourceRequest request) {
            String url = request.getUrl().toString();
            String scheme = request.getUrl().getScheme();
            if (!"http".equals(scheme) && !"https".equals(scheme)) return null;
            if (Native.verdict(url, null, Native.OTHER, Native.STANDARD) == Native.VERDICT_ALLOW) return null;
            Hooks.log("service-worker request refused " + url);
            return new WebResourceResponse("text/plain", "utf-8", 403, "Blocked",
                    Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
        }
    };

    static void interceptServiceWorkers() {
        ServiceWorkerController.getInstance().setServiceWorkerClient(SERVICE_WORKERS);
    }

    static void warm() {
        Thread thread = new Thread(() -> {
            long start = System.nanoTime();
            int rules = Native.warm();
            Hooks.log("rule lists built: " + rules + " rules in " + (System.nanoTime() - start) / 1000 + " us");
        }, "tarn-rules");
        thread.setPriority(Thread.MAX_PRIORITY);
        thread.start();
    }
}
