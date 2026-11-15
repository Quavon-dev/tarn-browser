package dev.tarn.android;

import android.os.Bundle;

public final class MainActivity extends ShellActivity {
    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        startShell(null, false);
    }
}
