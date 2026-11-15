package dev.tarn.android;

import android.content.Context;
import android.util.Log;
import android.webkit.CookieManager;
import android.webkit.WebView;

import androidx.webkit.Profile;
import androidx.webkit.ProfileStore;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

final class Stores {
    static final String DEFAULT = "";
    private static final String TAG = "tarn";
    private static final Pattern UUID =
            Pattern.compile("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$");
    private static final String PRIVATE_PREFIX = "private:";
    private static final String PROFILE = "tarn-";
    private static final String EPHEMERAL_PROFILE = "tarn-private-";
    private static final String BURNER_PREFIX = "burner:";
    private static final String BURNER_PROFILE = "tarn-burner-";
    private static final Map<String, Integer> USERS = new HashMap<>();
    private static final Set<String> RELEASED = new java.util.HashSet<>();

    private Stores() {}

    static String profileName(String store) {
        String session = Host.privateSession;
        if (store == null) return null;
        if (UUID.matcher(store).matches()) {
            if (session != null) return null;
            return multiProfile() ? PROFILE + store : null;
        }
        if (store.startsWith(BURNER_PREFIX)) {
            String id = store.substring(BURNER_PREFIX.length());
            if (!PrivateSpace.valid(id) || session != null || !multiProfile()) return null;
            return BURNER_PROFILE + id;
        }
        if (!store.startsWith(PRIVATE_PREFIX)) return null;
        String id = store.substring(PRIVATE_PREFIX.length());
        if (!PrivateSpace.valid(id)) return null;
        if (session != null) return session.equals(id) ? DEFAULT : null;
        return multiProfile() ? EPHEMERAL_PROFILE + id : null;
    }

    static boolean multiProfile() {
        return WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE);
    }

    static void attach(WebView view, String name) {
        if (name.isEmpty()) return;
        ProfileStore.getInstance().getOrCreateProfile(name).getServiceWorkerController()
                .setServiceWorkerClient(TarnApp.SERVICE_WORKERS);
        WebViewCompat.setProfile(view, name);
        USERS.merge(name, 1, Integer::sum);
    }

    static void detach(String name) {
        if (name.isEmpty()) return;
        USERS.computeIfPresent(name, (k, n) -> n > 1 ? n - 1 : null);
        if (!USERS.containsKey(name) && RELEASED.remove(name)) delete(name);
    }

    static void release(Context context, String session) {
        if (session == null || !PrivateSpace.valid(session) || Host.privateSession != null || !multiProfile()) return;
        for (String name : new String[] {EPHEMERAL_PROFILE + session, BURNER_PROFILE + session}) {
            if (USERS.containsKey(name)) {
                RELEASED.add(name);
            } else if (ProfileStore.getInstance().getProfile(name) != null) {
                delete(name);
            }
        }
    }

    static CookieManager cookies(String name) {
        if (name.isEmpty()) return CookieManager.getInstance();
        Profile profile = ProfileStore.getInstance().getProfile(name);
        return profile == null ? null : profile.getCookieManager();
    }

    static void wipe(Context context, String uuid) {
        if (context == null || uuid == null || !UUID.matcher(uuid).matches() || !multiProfile()) return;
        String name = PROFILE + uuid;
        if (!USERS.containsKey(name) && delete(name)) return;
        Set<String> pending = pending(context);
        pending.add(name);
        savePending(context, pending);
        Hooks.log("store wipe deferred to the next launch " + name);
    }

    static int importCookies(String store, String[] urls, String[] values) {
        String name = profileName(store);
        if (name == null || urls.length != values.length) return 0;
        if (!name.isEmpty()) ProfileStore.getInstance().getOrCreateProfile(name);
        CookieManager manager = cookies(name);
        if (manager == null) return 0;
        for (int i = 0; i < urls.length; i++) manager.setCookie(urls[i], values[i]);
        manager.flush();
        return urls.length;
    }

    static void cleanUpAtLaunch(Context context) {
        if (!multiProfile()) return;
        Set<String> pending = pending(context);
        Set<String> left = new LinkedHashSet<>();
        for (String name : pending) {
            if (!delete(name)) left.add(name);
        }
        for (String name : ProfileStore.getInstance().getAllProfileNames()) {
            boolean throwaway = name.startsWith(EPHEMERAL_PROFILE) || name.startsWith(BURNER_PROFILE);
            if (throwaway && !delete(name)) left.add(name);
        }
        savePending(context, left);
    }

    private static boolean delete(String name) {
        try {
            boolean deleted = ProfileStore.getInstance().deleteProfile(name);
            Hooks.log("profile " + name + " deleted=" + deleted);
            return true;
        } catch (IllegalStateException e) {
            Log.w(TAG, "profile in use, not deleted");
            return false;
        }
    }

    private static File pendingFile(Context context) {
        return new File(context.getNoBackupFilesDir(), "wipe-pending");
    }

    private static Set<String> pending(Context context) {
        Set<String> names = new LinkedHashSet<>();
        try {
            for (String line : Files.readAllLines(pendingFile(context).toPath(), StandardCharsets.UTF_8)) {
                if (line.startsWith(PROFILE)) names.add(line);
            }
        } catch (IOException ignored) {
            return names;
        }
        return names;
    }

    private static void savePending(Context context, Set<String> names) {
        File file = pendingFile(context);
        try {
            if (names.isEmpty()) {
                Files.deleteIfExists(file.toPath());
            } else {
                List<String> lines = new ArrayList<>(names);
                Files.write(file.toPath(), lines, StandardCharsets.UTF_8);
            }
        } catch (IOException e) {
            Log.e(TAG, "pending store wipes not saved", e);
        }
    }
}
