package dev.tarn.android;

import android.app.ActivityManager;
import android.content.Context;
import android.content.Intent;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;

import java.io.File;
import java.io.IOException;
import java.security.SecureRandom;
import java.util.List;
import java.util.regex.Pattern;

final class PrivateSpace {
    static final String EXTRA_SESSION = "dev.tarn.android.session";
    private static final String TAG = "tarn";
    private static final Pattern SESSION = Pattern.compile("^[0-9a-f]{32}$");
    private static final String PROCESS_SUFFIX = ":private";
    private static final String ORIGIN_VISITS = "shared_prefs/AwOriginVisitLoggerPrefs.xml";

    private PrivateSpace() {}

    static boolean valid(String session) {
        return session != null && SESSION.matcher(session).matches();
    }

    static String newSession() {
        byte[] bytes = new byte[16];
        new SecureRandom().nextBytes(bytes);
        StringBuilder hex = new StringBuilder();
        for (byte b : bytes) hex.append(String.format("%02x", b));
        return hex.toString();
    }

    static File root(Context context) {
        return new File(context.getDataDir(), "private");
    }

    static File sessionDir(Context context, String session) {
        return new File(root(context), session);
    }

    private static File tokens(Context context) {
        return new File(context.getNoBackupFilesDir(), "private-tokens");
    }

    static void open(Context context) {
        if (!running(context)) sweep(context, false);
        String session = newSession();
        if (!issueToken(context, session)) return;
        context.startActivity(new Intent(context, PrivateActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra(EXTRA_SESSION, session));
    }

    private static boolean issueToken(Context context, String session) {
        File dir = tokens(context);
        try {
            return (dir.isDirectory() || dir.mkdirs()) && new File(dir, session).createNewFile();
        } catch (IOException e) {
            Log.e(TAG, "private session token not written", e);
            return false;
        }
    }

    static boolean consumeToken(Context context, String session) {
        return valid(session) && new File(tokens(context), session).delete();
    }

    static boolean running(Context context) {
        ActivityManager am = context.getSystemService(ActivityManager.class);
        List<ActivityManager.RunningAppProcessInfo> processes = am.getRunningAppProcesses();
        if (processes == null) return false;
        String name = context.getPackageName() + PROCESS_SUFFIX;
        for (ActivityManager.RunningAppProcessInfo info : processes) {
            if (name.equals(info.processName)) return true;
        }
        return false;
    }

    static int sweep(Context context, boolean originVisits) {
        int count = deleteTree(root(context)) + deleteTree(tokens(context));
        if (originVisits && new File(context.getDataDir(), ORIGIN_VISITS).delete()) count++;
        Hooks.log("private sweep removed " + count + " files");
        return count;
    }

    static int sweepSession(Context context, String session) {
        if (!valid(session)) return 0;
        int count = deleteTree(sessionDir(context, session)) + deleteTree(new File(tokens(context), session));
        Hooks.log("private session swept, " + count + " files");
        return count;
    }

    private static int deleteTree(File file) {
        int count = 0;
        File[] children = file.isDirectory() && !java.nio.file.Files.isSymbolicLink(file.toPath())
                ? file.listFiles() : null;
        if (children != null) {
            for (File child : children) count += deleteTree(child);
        }
        if (file.exists() && file.delete()) count++;
        return count;
    }

    static void noPersonalizedLearning(View view) {
        if (view instanceof EditText edit) {
            int options = edit.getImeOptions();
            if ((options & EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING) == 0) {
                edit.setImeOptions(options | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
                if (edit.hasFocus()) {
                    edit.getContext().getSystemService(InputMethodManager.class).restartInput(edit);
                }
            }
        } else if (view instanceof ViewGroup group) {
            for (int i = 0; i < group.getChildCount(); i++) noPersonalizedLearning(group.getChildAt(i));
        }
    }
}
