package dev.tarn.android;

import android.app.Service;
import android.content.Intent;
import android.os.IBinder;

import java.io.File;

public final class SweepService extends Service {
    static final String EXTRA_PID = "dev.tarn.android.pid";
    static final String EXTRA_SESSION = "dev.tarn.android.sweep-session";
    private static final int WAIT_STEPS = 100;
    private static final long STEP_MS = 100;

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        int pid = intent == null ? -1 : intent.getIntExtra(EXTRA_PID, -1);
        String session = intent == null ? null : intent.getStringExtra(EXTRA_SESSION);
        if (pid <= 0 || !PrivateSpace.valid(session)) {
            stopSelf(startId);
            return START_NOT_STICKY;
        }
        new Thread(() -> {
            File proc = new File("/proc/" + pid);
            for (int i = 0; i < WAIT_STEPS && proc.exists(); i++) {
                try {
                    Thread.sleep(STEP_MS);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    break;
                }
            }
            if (!proc.exists()) PrivateSpace.sweepSession(this, session);
            Host.MAIN.post(() -> stopSelf(startId));
        }, "tarn-sweep").start();
        return START_NOT_STICKY;
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
