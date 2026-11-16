package dev.tarn.android;

import android.net.Uri;
import android.webkit.WebView;

import androidx.webkit.JavaScriptExecutionException;
import androidx.webkit.JavaScriptExecutionWorld;
import androidx.webkit.JavaScriptReplyProxy;
import androidx.webkit.ScriptHandler;
import androidx.webkit.WebMessageCompat;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import androidx.webkit.WebViewOutcomeReceiver;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;

final class Extensions {
    static final String PORT = "__tarnExtPort";
    static final int MAX_MESSAGE_CHARS = 1 << 20;
    private static final String START = "start ";
    private static final List<String> PHASES = List.of("start", "end", "idle");
    private static final int MAX_PENDING = 256;
    private static final Set<String> ANY = Set.of("*");
    private static final WebViewOutcomeReceiver<String, JavaScriptExecutionException> IGNORED = new WebViewOutcomeReceiver<>() {
        @Override
        public void onResult(String result) {}

        @Override
        public void onError(JavaScriptExecutionException error) {
            Hooks.log("extension script not run: " + error.getErrorType());
        }
    };
    private static final Map<String, Pending> PENDING = new LinkedHashMap<>() {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, Pending> eldest) {
            return size() > MAX_PENDING;
        }
    };
    private static long nextRequest = 1;

    interface Owner {
        String tab();

        String store();

        boolean runs();

        void frame(Map<String, Object> request);

        void message(String world, long frame, String origin, String json);
    }

    private record Registered(JavaScriptExecutionWorld world, ScriptHandler loader) {}

    private record Pending(Extensions view, String world, long frame, long generation) {}

    private static final class Frames {
        final Map<JavaScriptReplyProxy, Long> ids = new IdentityHashMap<>();
        final Map<Long, JavaScriptReplyProxy> proxies = new HashMap<>();
        final Map<Long, Long> generations = new HashMap<>();
        final Set<Long> approved = new HashSet<>();
        final Map<Long, Set<String>> reached = new HashMap<>();
        final Map<Long, Map<String, List<String>>> later = new HashMap<>();
        String mainUrl = "";

        void reset() {
            ids.clear();
            proxies.clear();
            generations.clear();
            approved.clear();
            reached.clear();
            later.clear();
        }
    }

    private final WebView web;
    private final Owner owner;
    private final Map<String, Registered> worlds = new HashMap<>();
    private final Map<String, Frames> frames = new HashMap<>();
    private long nextFrame = 1;
    private long nextGeneration = 1;
    private String applied;

    Extensions(WebView web, Owner owner) {
        this.web = web;
        this.owner = owner;
    }

    static boolean worldsSupported() {
        return WebViewFeature.isFeatureSupported(WebViewFeature.JS_INJECTION_IN_FRAME_AND_WORLD)
                && WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER);
    }

    static void ready(String request, String[] units) {
        Pending pending = PENDING.remove(request);
        if (pending != null) pending.view().deliver(pending, units);
    }

    void apply() {
        String store = owner.store();
        String[] names = store == null || !worldsSupported() ? new String[0] : Native.extensionWorlds(store);
        String key = names == null ? "" : String.join(",", names);
        if (key.equals(applied)) return;
        clear();
        applied = key;
        Hooks.log("extension worlds for " + store + ": " + key);
        if (names == null || names.length == 0) return;
        String loader = Native.extensionLoader();
        for (String name : names) register(name, loader);
    }

    private void register(String name, String loader) {
        try {
            JavaScriptExecutionWorld world = WebViewCompat.getExecutionWorld(web, name);
            ScriptHandler handler = WebViewCompat.addJavaScriptOnEvent(web, loader, WebViewCompat.INJECTION_EVENT_DOCUMENT_START, ANY, world);
            try {
                WebViewCompat.addWebMessageListener(web, PORT, ANY, world,
                        (view, message, origin, main, reply) -> received(name, view, message, origin, main, reply));
            } catch (RuntimeException e) {
                handler.remove();
                throw e;
            }
            worlds.put(name, new Registered(world, handler));
        } catch (IllegalArgumentException | UnsupportedOperationException e) {
            Hooks.log("extension world not added " + name);
        }
    }

    void clear() {
        for (Registered registered : worlds.values()) {
            registered.loader().remove();
            WebViewCompat.removeWebMessageListener(web, registered.world(), PORT);
        }
        worlds.clear();
        frames.clear();
        applied = null;
    }

    void post(String world, Long frame, String json) {
        Frames known = frames.get(world);
        if (known == null || json == null || !worlds.containsKey(world)) return;
        List<Long> targets = frame == null ? new ArrayList<>(known.approved) : List.of(frame);
        for (Long id : targets) {
            JavaScriptReplyProxy proxy = known.proxies.get(id);
            if (proxy != null && known.approved.contains(id)) proxy.postMessage(json);
        }
    }

    private void received(String name, WebView view, WebMessageCompat message, Uri origin, boolean main, JavaScriptReplyProxy reply) {
        if (view != web || !worlds.containsKey(name) || message.getType() != WebMessageCompat.TYPE_STRING) return;
        String data = message.getData();
        if (data == null || data.length() > MAX_MESSAGE_CHARS) return;
        Frames known = frames.computeIfAbsent(name, k -> new Frames());
        if (data.startsWith(START)) {
            start(name, known, data.substring(START.length()), origin, main, reply);
            return;
        }
        Long id = known.ids.get(reply);
        if (id == null) return;
        if (data.startsWith("{")) {
            if (known.approved.contains(id)) owner.message(name, id, main || origin == null ? "" : origin.toString(), data);
            return;
        }
        if (!PHASES.contains(data)) return;
        known.reached.computeIfAbsent(id, k -> new HashSet<>()).add(data);
        Map<String, List<String>> due = known.later.get(id);
        List<String> scripts = due == null ? null : due.remove(data);
        if (scripts != null) for (String code : scripts) reply.executeJavaScript(code, IGNORED);
    }

    private void start(String name, Frames known, String url, Uri origin, boolean main, JavaScriptReplyProxy reply) {
        long id;
        if (main) {
            known.reset();
            known.mainUrl = url;
            id = 0;
        } else {
            Long seen = known.ids.get(reply);
            id = seen != null ? seen : nextFrame++;
        }
        long generation = nextGeneration++;
        known.ids.put(reply, id);
        known.proxies.put(id, reply);
        known.generations.put(id, generation);
        known.approved.remove(id);
        known.reached.remove(id);
        known.later.remove(id);
        String store = owner.store();
        if (!owner.runs() || store == null || origin == null) return;
        String request = String.valueOf(nextRequest++);
        PENDING.put(request, new Pending(this, name, id, generation));
        Map<String, Object> event = new LinkedHashMap<>();
        event.put("request", request);
        event.put("tab", owner.tab());
        event.put("store", store);
        event.put("world", name);
        event.put("url", url);
        event.put("top", main ? "" : known.mainUrl);
        event.put("main", main);
        event.put("origin", origin.toString());
        owner.frame(event);
    }

    private void deliver(Pending pending, String[] units) {
        Frames known = frames.get(pending.world());
        if (known == null || !worlds.containsKey(pending.world())
                || !Objects.equals(known.generations.get(pending.frame()), pending.generation())) {
            return;
        }
        JavaScriptReplyProxy proxy = known.proxies.get(pending.frame());
        Hooks.log("extension frame " + pending.frame() + " of " + pending.world() + ": " + (units == null ? 0 : units.length / 2) + " scripts");
        if (proxy == null || units == null || units.length < 2) return;
        known.approved.add(pending.frame());
        Set<String> reached = known.reached.getOrDefault(pending.frame(), Set.of());
        Map<String, List<String>> later = new HashMap<>();
        for (String phase : PHASES) {
            for (int i = 0; i + 1 < units.length; i += 2) {
                if (!phase.equals(units[i])) continue;
                if (PHASES.get(0).equals(phase) || reached.contains(phase)) {
                    proxy.executeJavaScript(units[i + 1], IGNORED);
                } else {
                    later.computeIfAbsent(phase, k -> new ArrayList<>()).add(units[i + 1]);
                }
            }
        }
        if (!later.isEmpty()) known.later.put(pending.frame(), later);
    }
}
