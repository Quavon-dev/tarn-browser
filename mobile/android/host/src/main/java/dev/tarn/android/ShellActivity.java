package dev.tarn.android;

import android.app.Activity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.util.Log;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;

import com.longbridge.nsh.NshView;

import java.io.File;

abstract class ShellActivity extends Activity {
    private static final String TAG = "tarn";
    private static final int CHOOSE_FILES = 1;
    NshView nsh;
    private ValueCallback<Uri[]> chosen;

    void startShell(File storage, boolean noPersonalizedLearning) {
        nsh = new NshView(this);
        nsh.setErrorListener(message -> Log.e(TAG, message));
        if (storage != null) nsh.setStorageDirectory(storage);
        nsh.registerComponent("tarn.webview", context -> new TarnWebView(context, noPersonalizedLearning));
        setContentView(nsh);
        nsh.runRustApp("nsh");
    }

    void destroyShell() {
        if (nsh == null) return;
        nsh.destroy();
        nsh = null;
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        if (nsh == null || !nsh.handleBack()) super.onBackPressed();
    }

    @Override
    protected void onResume() {
        super.onResume();
        Host.activity = this;
    }

    boolean chooseFiles(ValueCallback<Uri[]> callback, WebChromeClient.FileChooserParams params) {
        if (chosen != null) chosen.onReceiveValue(null);
        chosen = callback;
        Intent intent = params.createIntent();
        if (params.getMode() == WebChromeClient.FileChooserParams.MODE_OPEN_MULTIPLE) {
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        }
        try {
            startActivityForResult(intent, CHOOSE_FILES);
            return true;
        } catch (ActivityNotFoundException e) {
            chosen = null;
            return false;
        }
    }

    @Override
    @SuppressWarnings("deprecation")
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != CHOOSE_FILES || chosen == null) {
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        ValueCallback<Uri[]> callback = chosen;
        chosen = null;
        ClipData clip = resultCode == RESULT_OK && data != null ? data.getClipData() : null;
        if (clip == null) {
            callback.onReceiveValue(WebChromeClient.FileChooserParams.parseResult(resultCode, data));
            return;
        }
        Uri[] uris = new Uri[clip.getItemCount()];
        for (int i = 0; i < uris.length; i++) uris[i] = clip.getItemAt(i).getUri();
        callback.onReceiveValue(uris);
    }

    @Override
    protected void onDestroy() {
        if (chosen != null) chosen.onReceiveValue(null);
        chosen = null;
        if (Host.activity == this) Host.activity = null;
        destroyShell();
        super.onDestroy();
    }
}
