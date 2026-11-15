package dev.tarn.android;

import android.content.Intent;
import android.os.Build;
import android.os.Bundle;
import android.os.Process;
import android.util.Log;
import android.view.WindowManager;

import androidx.webkit.ProcessGlobalConfig;
import androidx.webkit.WebViewFeature;

import java.io.File;

public final class PrivateActivity extends ShellActivity {
    private static final String TAG = "tarn";
    private static String session;
    private boolean closing;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        String requested = getIntent().getStringExtra(PrivateSpace.EXTRA_SESSION);
        if (session == null) {
            if (!PrivateSpace.consumeToken(this, requested) || !configure(requested)) {
                Log.w(TAG, "private space not resumed: no fresh session");
                close(requested);
                return;
            }
            session = requested;
        } else if (requested != null && !session.equals(requested)) {
            close(session);
            return;
        }
        Host.privateActivity = this;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            setRecentsScreenshotEnabled(false);
        } else {
            getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        }
        startShell(new File(PrivateSpace.sessionDir(this, session), "nsh"), true);
        nsh.getViewTreeObserver().addOnGlobalLayoutListener(() -> {
            if (nsh != null) PrivateSpace.noPersonalizedLearning(nsh);
        });
    }

    private boolean configure(String id) {
        if (!WebViewFeature.isStartupFeatureSupported(this, WebViewFeature.STARTUP_FEATURE_SET_DIRECTORY_BASE_PATHS)) {
            Log.e(TAG, "WebView cannot keep the private space in its own directory");
            return false;
        }
        File dir = PrivateSpace.sessionDir(this, id);
        try {
            ProcessGlobalConfig config = new ProcessGlobalConfig();
            config.setDirectoryBasePaths(this, new File(dir, "data"), new File(dir, "cache"));
            ProcessGlobalConfig.apply(config);
        } catch (IllegalStateException | IllegalArgumentException e) {
            Log.e(TAG, "private WebView directory not applied", e);
            return false;
        }
        Host.privateSession = id;
        if (!Native.start(Host.class, new File(dir, "files").getPath(), id)) return false;
        TarnApp.warm();
        TarnApp.interceptServiceWorkers();
        Hooks.configure(this);
        return true;
    }

    void close() {
        close(session);
    }

    private void close(String sweepSession) {
        if (closing) return;
        closing = true;
        destroyShell();
        Host.privateActivity = null;
        Intent sweep = new Intent(this, SweepService.class)
                .putExtra(SweepService.EXTRA_PID, Process.myPid())
                .putExtra(SweepService.EXTRA_SESSION, sweepSession);
        try {
            if (PrivateSpace.valid(sweepSession)) startService(sweep);
        } catch (IllegalStateException e) {
            Log.w(TAG, "sweep left to the next launch", e);
        }
        finishAndRemoveTask();
        Process.killProcess(Process.myPid());
    }

    @Override
    protected void onDestroy() {
        if (isFinishing()) {
            close();
        }
        super.onDestroy();
    }
}
