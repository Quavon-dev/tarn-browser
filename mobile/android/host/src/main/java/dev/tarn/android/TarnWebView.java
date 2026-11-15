package dev.tarn.android;

import android.graphics.Bitmap;
import android.net.Uri;
import android.net.http.SslError;
import android.os.Message;
import android.os.SystemClock;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.CookieManager;
import android.webkit.GeolocationPermissions;
import android.webkit.HttpAuthHandler;
import android.webkit.JsPromptResult;
import android.webkit.JsResult;
import android.webkit.PermissionRequest;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.SslErrorHandler;
import android.webkit.ValueCallback;
import android.webkit.WebBackForwardList;
import android.webkit.WebChromeClient;
import android.webkit.WebHistoryItem;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;

import androidx.webkit.ScriptHandler;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;

import com.longbridge.nsh.NshComponent;
import com.longbridge.nsh.NshComponentContext;

import java.io.ByteArrayInputStream;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.TreeSet;
import java.util.function.Consumer;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONTokener;

final class TarnWebView implements NshComponent {
    private record Page(String url, int level, String from) {}

    private record PageText(String url, String text) {}

    private static final int MIN_ZOOM = 50;
    private static final int MAX_ZOOM = 300;
    private static final long POPUP_WAIT_MS = 5_000;
    private static final int POPUP_BURST = Native.popupBurst();
    private static final int DOWNLOAD_BURST = Native.downloadBurst();
    private static final long BURST_WINDOW_MS = Native.burstWindowMs();
    private static final long POLL_MS = 100;
    private static final long SCRIPT_WAIT_MS = 10_000;
    private static final String CANCEL_PICK = "if (window.tarnPick) window.tarnPick();";
    private static final java.security.SecureRandom RANDOM = new java.security.SecureRandom();
    private static final long FIND_CACHE_MS = 60_000;
    private static final java.util.concurrent.ExecutorService FINDER = java.util.concurrent.Executors.newSingleThreadExecutor();
    private static final Set<TarnWebView> LIVE = new java.util.LinkedHashSet<>();
    private static final List<Map<String, Object>> UNSENT = new ArrayList<>();

    private final ArrayDeque<Long> popups = new ArrayDeque<>();
    private final ArrayDeque<Long> downloads = new ArrayDeque<>();
    private final Map<String, Integer> authAttempts = new HashMap<>();

    private final NshComponentContext context;
    private final FrameLayout container;
    private final boolean noPersonalizedLearning;
    private final Prompts prompts = new Prompts();
    private final Set<String> upgraded = Collections.synchronizedSet(new HashSet<>());
    private final Object navigation = new Object();

    private PageView web;
    private String tab = "";
    private String store;
    private String profileName;
    private String builtStore;
    private String loadUrl;
    private String loadOpener;
    private Object loadSeq;
    private Object appliedSeq;
    private boolean active = true;
    private boolean applyPosted;
    private boolean destroyed;
    private boolean jsDialogOpen;
    private volatile String refusedUrl;
    private String findText = "";
    private int textZoom = 100;
    private Map<String, String> styles = Map.of();
    private String themeStyles = "[]";
    private Map<String, List<String>> zaps = Map.of();
    private boolean sortDownloads;
    private String appliedContent;
    private final List<ScriptHandler> contentScripts = new ArrayList<>();
    private String pickToken;
    private Consumer<String> pickDone;
    private PageText findCache;
    private long findUsed;

    private volatile int level = Native.STANDARD;
    private volatile Map<String, Integer> levels = Map.of();
    private volatile int viewLevel = Native.STANDARD;
    private volatile Page top;
    private volatile Page pending;
    private volatile boolean shellNavigation;
    private volatile boolean javaScriptOn;

    TarnWebView(NshComponentContext context, boolean noPersonalizedLearning) {
        this.context = context;
        this.noPersonalizedLearning = noPersonalizedLearning;
        container = new FrameLayout(context.getContext());
        container.setContentDescription("Web content");
        LIVE.add(this);
    }

    @Override
    public View view() {
        return container;
    }

    @Override
    public void setProp(String name, Object value) {
        switch (name) {
            case "tab" -> tab = value instanceof String s ? s : "";
            case "profile" -> store = value instanceof String s ? s : null;
            case "level" -> level = Native.level(value);
            case "levels" -> levels = parseLevels(value);
            case "active" -> active = !Boolean.FALSE.equals(value);
            case "styles" -> styles = parseStyles(value);
            case "themeStyles" -> themeStyles = value instanceof List<?> list ? new JSONArray(list).toString() : "[]";
            case "zaps" -> zaps = parseZaps(value);
            case "sortDownloads" -> sortDownloads = Boolean.TRUE.equals(value);
            case "load" -> {
                if (value instanceof Map<?, ?> load && load.get("url") instanceof String url) {
                    loadUrl = url;
                    loadSeq = load.get("seq");
                    loadOpener = load.get("opener") instanceof String opener ? opener : null;
                }
            }
            default -> {
                return;
            }
        }
        if (!applyPosted) {
            applyPosted = true;
            container.post(this::apply);
        }
    }

    @Override
    public void command(String name, Object args) {
        Map<?, ?> map = args instanceof Map<?, ?> m ? m : Map.of();
        if ("answer".equals(name)) {
            prompts.answer(map);
            return;
        }
        if (transferCommand(name, map) || scriptCommand(name, map)) return;
        if (web == null) {
            if ("reload".equals(name) && loadUrl != null) {
                appliedSeq = null;
                apply();
            }
            return;
        }
        switch (name) {
            case "back" -> step(-1);
            case "forward" -> step(1);
            case "reload" -> reload();
            case "stop" -> {
                web.stopLoading();
                endNavigation();
            }
            case "find" -> find(Prompts.text(map, "text"), Boolean.TRUE.equals(map.get("backwards")));
            case "zoom" -> zoom(map.get("factor"));
            default -> {
                if (!Hooks.command(name, web)) Hooks.log("unknown command " + name);
            }
        }
    }

    @Override
    public void destroy() {
        destroyed = true;
        LIVE.remove(this);
        tearDown();
    }

    private static final String DEFAULT_LEVEL_KEY = "*";

    private static Map<String, String> parseStyles(Object value) {
        if (!(value instanceof Map<?, ?> map)) return Map.of();
        Map<String, String> parsed = new HashMap<>();
        for (Map.Entry<?, ?> entry : map.entrySet()) {
            if (entry.getKey() instanceof String host && entry.getValue() instanceof String css) parsed.put(normalHost(host), css);
        }
        return Map.copyOf(parsed);
    }

    private static Map<String, List<String>> parseZaps(Object value) {
        if (!(value instanceof Map<?, ?> map)) return Map.of();
        Map<String, List<String>> parsed = new HashMap<>();
        for (Map.Entry<?, ?> entry : map.entrySet()) {
            if (!(entry.getKey() instanceof String host) || !(entry.getValue() instanceof List<?> list)) continue;
            List<String> selectors = new ArrayList<>();
            for (Object item : list) if (item instanceof String selector) selectors.add(selector);
            parsed.put(normalHost(host), List.copyOf(selectors));
        }
        return Map.copyOf(parsed);
    }

    private void applyContent() {
        String key = styles + "|" + zaps + "|" + themeStyles;
        if (web == null || key.equals(appliedContent)) return;
        for (ScriptHandler handler : contentScripts) handler.remove();
        contentScripts.clear();
        appliedContent = key;
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) return;
        String themeScript = Native.themeStylesScript(themeStyles);
        if (themeScript != null) {
            try {
                contentScripts.add(WebViewCompat.addDocumentStartJavaScript(web, themeScript, Set.of(Native.themeOrigins(themeStyles))));
            } catch (IllegalArgumentException e) {
                Hooks.log("no theme style sheet");
            }
        }
        Set<String> hosts = new TreeSet<>(styles.keySet());
        hosts.addAll(zaps.keySet());
        for (String host : hosts) {
            String script = Native.siteStylesScript(host, styles.getOrDefault(host, ""),
                    zaps.getOrDefault(host, List.of()).toArray(new String[0]), themeStyles);
            if (script == null) continue;
            try {
                contentScripts.add(WebViewCompat.addDocumentStartJavaScript(web, script, Set.of("https://" + host, "http://" + host)));
            } catch (IllegalArgumentException e) {
                Hooks.log("no style sheet for host " + host);
            }
        }
    }

    private boolean locked() {
        Page page = top;
        return web == null || !javaScriptOn || page == null || levelFor(page.url()) == Native.LOCKDOWN;
    }

    private static String decoded(String json) {
        if (json == null) return null;
        try {
            return new JSONTokener(json).nextValue() instanceof String text ? text : null;
        } catch (JSONException e) {
            return null;
        }
    }

    private void runSync(String body, Consumer<String> done) {
        if (body == null || locked()) {
            done.accept("");
            return;
        }
        web.evaluateJavascript("(() => {" + body + "})()", result -> {
            String text = decoded(result);
            done.accept(text == null ? "" : text);
        });
    }

    private String runAsync(String body, long waitMs, Consumer<String> done) {
        if (body == null || locked()) {
            done.accept("");
            return null;
        }
        String key = "t" + new java.math.BigInteger(128, RANDOM).toString(16);
        web.evaluateJavascript("(async () => {" + body + "})().then(r => { window['" + key + "'] = String(r); }, () => { window['" + key + "'] = ''; })", null);
        poll(web, key, waitMs == Long.MAX_VALUE ? -1 : SystemClock.elapsedRealtime() + waitMs, done);
        return key;
    }

    private void poll(WebView view, String key, long deadline, Consumer<String> done) {
        container.postDelayed(() -> {
            boolean picker = deadline < 0;
            if (destroyed || web != view || picker && !key.equals(pickToken)) {
                if (!picker) done.accept("");
                return;
            }
            view.evaluateJavascript("(() => { const r = window['" + key + "']; delete window['" + key + "']; return r; })()", result -> {
                String text = decoded(result);
                if (text != null) {
                    done.accept(text);
                } else if (picker || SystemClock.elapsedRealtime() < deadline) {
                    poll(view, key, deadline, done);
                } else {
                    done.accept("");
                }
            });
        }, POLL_MS);
    }

    private boolean scriptCommand(String name, Map<?, ?> args) {
        Object seq = args.get("seq");
        Page page = top;
        String url = page == null ? "" : page.url();
        switch (name) {
            case "readArticle" -> {
                String body = page == null ? null : Native.readerScript(url, Boolean.TRUE.equals(args.get("probe")));
                runAsync(body, SCRIPT_WAIT_MS, text -> emit("article", "seq", seq, "url", url, "text", text));
            }
            case "readText" -> {
                int max = args.get("maxChars") instanceof Number n ? n.intValue() : 0;
                runSync(page == null ? null : Native.pageTextScript(url, max), text -> emit("pageText", "seq", seq, "url", url, "text", text));
            }
            case "findText" -> {
                String query = Prompts.text(args, "query");
                Consumer<String> search = text -> FINDER.execute(() -> {
                    String[] found = text.isEmpty() ? new String[] {"0"} : Native.findSnippets(text, query);
                    List<String> snippets = found.length > 1 ? List.of(java.util.Arrays.copyOfRange(found, 1, found.length)) : List.of();
                    int matched = found.length > 0 ? Integer.parseInt(found[0]) : 0;
                    emit("textFound", "seq", seq, "url", url, "snippets", snippets, "matched", matched);
                });
                PageText cached = findCache;
                keepFindCache();
                if (cached != null && cached.url().equals(url) && query != null && !query.isEmpty()) {
                    search.accept(cached.text());
                    return true;
                }
                String body = page == null || query == null || query.isEmpty() ? null : Native.findTextScript(url);
                runSync(body, text -> {
                    if (body != null && top == page) findCache = new PageText(url, text);
                    search.accept(text);
                });
            }
            case "pick" -> {
                endPick();
                Consumer<String> done = new Consumer<>() {
                    @Override
                    public void accept(String selector) {
                        if (pickDone != this) return;
                        pickDone = null;
                        pickToken = null;
                        emit("picked", "seq", seq, "url", url, "selector", selector);
                    }
                };
                pickDone = done;
                pickToken = runAsync(page == null ? null : Native.picker(), Long.MAX_VALUE, done);
            }
            case "pickCancel" -> {
                if (web != null && javaScriptOn) web.evaluateJavascript(CANCEL_PICK, null);
                endPick();
            }
            default -> {
                return false;
            }
        }
        return true;
    }

    private void keepFindCache() {
        long now = SystemClock.elapsedRealtime();
        findUsed = now;
        container.postDelayed(() -> {
            if (findUsed == now) findCache = null;
        }, FIND_CACHE_MS);
    }

    private void endPick() {
        Consumer<String> done = pickDone;
        if (done != null) done.accept("");
    }

    private boolean transferCommand(String name, Map<?, ?> args) {
        if (!name.startsWith("download")) return false;
        Downloads.Transfer transfer = Downloads.find(Prompts.text(args, "id"));
        if (transfer == null) return true;
        switch (name) {
            case "downloadPause" -> transfer.pause();
            case "downloadResume" -> transfer.resume();
            case "downloadCancel", "downloadKeep" -> {
                if (!"downloadKeep".equals(name) || !Boolean.TRUE.equals(args.get("keep"))) {
                    transfer.cancel();
                }
            }
            default -> {
                return false;
            }
        }
        return true;
    }

    private static Map<String, Integer> parseLevels(Object value) {
        if (!(value instanceof Map<?, ?> map)) return Map.of();
        Map<String, Integer> parsed = new HashMap<>();
        for (Map.Entry<?, ?> entry : map.entrySet()) {
            if (!(entry.getKey() instanceof String host)) continue;
            String key = DEFAULT_LEVEL_KEY.equals(host) ? host : normalHost(host);
            if (DEFAULT_LEVEL_KEY.equals(key) && !DEFAULT_LEVEL_KEY.equals(host)) continue;
            parsed.put(key, Native.level(entry.getValue()));
        }
        return Map.copyOf(parsed);
    }

    private static String normalHost(String host) {
        String lower = host.toLowerCase(Locale.ROOT);
        return lower.endsWith(".") ? lower.substring(0, lower.length() - 1) : lower;
    }

    private int levelFor(String url) {
        String host = url == null ? null : Uri.parse(url).getHost();
        Integer site = host == null ? null : levels.get(normalHost(host));
        if (site != null) return site;
        Integer fallback = levels.get(DEFAULT_LEVEL_KEY);
        return fallback == null ? level : fallback;
    }

    private void apply() {
        applyPosted = false;
        if (destroyed) return;
        if (!UNSENT.isEmpty()) {
            List<Map<String, Object>> held = new ArrayList<>(UNSENT);
            UNSENT.clear();
            for (Map<String, Object> data : held) emitMap("download", data);
        }
        if (web == null || !Objects.equals(store, builtStore)) {
            if (!build()) return;
        }
        if (active) {
            web.onResume();
        } else {
            web.onPause();
        }
        applyContent();
        if (loadUrl != null && !Objects.equals(loadSeq, appliedSeq)) {
            appliedSeq = loadSeq;
            shellLoad(loadUrl, loadOpener);
        }
    }

    private boolean build() {
        tearDown();
        String name = Stores.profileName(store);
        if (name == null) {
            emit("failed", "url", loadUrl == null ? "" : loadUrl, "message", "This profile's data store is not available here");
            return false;
        }
        PageView view = new PageView(context.getContext(), noPersonalizedLearning);
        Stores.attach(view, name);
        configure(view, name);
        container.addView(view, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        web = view;
        profileName = name;
        builtStore = store;
        viewLevel = level;
        appliedSeq = null;
        Hooks.log("tab " + tab + " web view on store " + (name.isEmpty() ? "process default" : name));
        return true;
    }

    private void configure(PageView view, String name) {
        WebView.setWebContentsDebuggingEnabled(false);
        WebSettings settings = view.getSettings();
        javaScriptOn = level != Native.LOCKDOWN;
        settings.setJavaScriptEnabled(javaScriptOn);
        settings.setJavaScriptCanOpenWindowsAutomatically(false);
        settings.setSupportMultipleWindows(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setGeolocationEnabled(true);
        settings.setSupportZoom(true);
        settings.setBuiltInZoomControls(true);
        settings.setDisplayZoomControls(false);
        settings.setUseWideViewPort(true);
        settings.setLoadWithOverviewMode(true);
        settings.setTextZoom(textZoom);
        CookieManager cookies = Stores.cookies(name);
        if (cookies != null) cookies.setAcceptThirdPartyCookies(view, false);
        view.setWebViewClient(new Client());
        view.setWebChromeClient(new Chrome());
        view.setDownloadListener(this::download);
        view.setFindListener((activeMatch, matches, done) -> {
            if (!done) return;
            emit("found", "matched", matches);
        });
    }

    private void tearDown() {
        prompts.declineAll();
        jsDialogOpen = false;
        endPick();
        for (ScriptHandler handler : contentScripts) handler.remove();
        contentScripts.clear();
        appliedContent = null;
        findCache = null;
        if (web == null) return;
        PageView old = web;
        web = null;
        container.removeView(old);
        old.destroy();
        Stores.detach(profileName);
        profileName = null;
        builtStore = null;
        top = null;
        pending = null;
        upgraded.clear();
    }

    private void setJavaScript(int action) {
        javaScriptOn = action == Native.CONTINUE_JS;
        web.getSettings().setJavaScriptEnabled(javaScriptOn);
    }

    private boolean admitPopup(long now) {
        return admit(popups, POPUP_BURST, now);
    }

    private static boolean admit(ArrayDeque<Long> recent, int max, long now) {
        while (!recent.isEmpty() && now - recent.peekFirst() >= BURST_WINDOW_MS) recent.pollFirst();
        if (recent.size() >= max) return false;
        recent.addLast(now);
        return true;
    }

    private boolean leavesJavaScriptOff(int action) {
        return !javaScriptOn && action == Native.CONTINUE_JS && (top != null || pending != null);
    }

    private boolean rebuildKeeping(int keptLevel) {
        Object seq = appliedSeq;
        if (!build()) return false;
        applyContent();
        appliedSeq = seq;
        viewLevel = keptLevel;
        if (active) {
            web.onResume();
        } else {
            web.onPause();
        }
        return true;
    }

    private void shellLoad(String target, String opener) {
        refusedUrl = null;
        if (!passesNavPolicy(target, opener == null ? Native.TYPED : Native.POPUP, opener)) return;
        int site = levelFor(target);
        if (opener != null && isWeb(target) && Native.intoPrivateNetwork(target, opener, false, site)) {
            blockLocal(target);
            return;
        }
        if (Native.levelAction(target, site, viewLevel, true) == Native.REBUILD) viewLevel = site;
        int action = Native.levelAction(target, site, viewLevel, true);
        if (action == Native.BLOCK_INSECURE) {
            block(target, "https_only");
            return;
        }
        String allowed = documentVerdict(target, site);
        if (allowed == null) return;
        if (leavesJavaScriptOff(action)) {
            if (!rebuildKeeping(viewLevel)) return;
            if (!allowed.equals(target)) upgraded.add(allowed);
        }
        setJavaScript(action);
        startNavigation(allowed, site, opener == null, opener);
        web.loadUrl(allowed);
    }

    private void startNavigation(String url, int site, boolean userStarted, String from) {
        synchronized (navigation) {
            pending = url == null ? null : new Page(url, site, from);
            shellNavigation = userStarted && url != null;
        }
    }

    private String origin() {
        Page next = pending;
        if (next != null) return next.from();
        Page current = top;
        return current == null ? null : current.url();
    }

    private void endNavigation() {
        synchronized (navigation) {
            pending = null;
            shellNavigation = false;
        }
    }

    private boolean passesNavPolicy(String target, int source, String from) {
        int decision = Native.decide(target, source);
        if (decision == Native.PROMPT) {
            emit("external", "url", target, "origin", origin(from));
            return false;
        }
        if (decision != Native.ALLOW) {
            block(target, "nav_policy");
            return false;
        }
        return true;
    }

    private String documentVerdict(String target, int site) {
        if (!isWeb(target)) return target;
        int verdict = Native.verdict(target, target, Native.MAIN_FRAME, site);
        if (verdict == Native.VERDICT_BLOCK) {
            block(target, "shield");
            return null;
        }
        if (verdict == Native.VERDICT_MAKE_HTTPS) {
            String secure = "https" + target.substring("http".length());
            if (!upgraded.add(secure)) {
                block(target, "https_only");
                return null;
            }
            Hooks.log("upgraded " + target + " -> " + secure);
            emit("shielded", "kind", "https_upgrade", "url", secure);
            return secure;
        }
        return target;
    }

    private static boolean isWeb(String url) {
        String scheme = Uri.parse(url).getScheme();
        return "http".equals(scheme) || "https".equals(scheme);
    }

    private boolean pageNavigation(String target, boolean redirect) {
        Page current = top;
        String from = redirect ? origin() : current == null ? null : current.url();
        if (!passesNavPolicy(target, Native.LINK, from)) return false;
        int site = levelFor(target);
        int action = Native.levelAction(target, site, viewLevel, false);
        if (action == Native.BLOCK_INSECURE) {
            block(target, "https_only");
            return false;
        }
        boolean userStarted = redirect && shellNavigation;
        int pageLevel = Math.max(site, viewLevel);
        if (isWeb(target) && Native.intoPrivateNetwork(target, from, userStarted, pageLevel)) {
            blockLocal(target);
            return false;
        }
        String allowed = documentVerdict(target, pageLevel);
        if (allowed == null) return false;
        setJavaScript(leavesJavaScriptOff(action) ? Native.CONTINUE_NO_JS : action);
        viewLevel = pageLevel;
        if (!allowed.equals(target)) {
            startNavigation(allowed, pageLevel, userStarted, from);
            web.loadUrl(allowed);
            return false;
        }
        startNavigation(target, pageLevel, userStarted, from);
        return true;
    }

    private void step(int offset) {
        if (!web.canGoBackOrForward(offset)) return;
        String url = historyUrl(offset);
        Page current = top;
        if (url == null || current != null && withoutFragment(url).equals(withoutFragment(current.url()))) {
            endNavigation();
            web.goBackOrForward(offset);
            return;
        }
        int site = levelFor(url);
        if (isWeb(url)) {
            int verdict = Native.verdict(url, url, Native.MAIN_FRAME, site);
            if (verdict != Native.VERDICT_ALLOW) {
                block(url, verdict == Native.VERDICT_BLOCK ? "shield" : "https_only");
                return;
            }
        }
        viewLevel = site;
        int action = Native.levelAction(url, site, site, true);
        if (leavesJavaScriptOff(action)) {
            if (!rebuildKeeping(site)) return;
            setJavaScript(action);
            startNavigation(url, site, true, null);
            web.loadUrl(url);
            return;
        }
        setJavaScript(action);
        startNavigation(url, site, true, current == null ? null : current.url());
        web.goBackOrForward(offset);
    }

    private static String withoutFragment(String url) {
        int hash = url.indexOf('#');
        return hash < 0 ? url : url.substring(0, hash);
    }

    private void reload() {
        String url = web.getUrl();
        if (url == null && loadUrl != null) {
            appliedSeq = null;
            apply();
            return;
        }
        Page current = top;
        startNavigation(url, current == null ? levelFor(url) : current.level(), true, url);
        web.reload();
    }

    private String historyUrl(int offset) {
        WebBackForwardList list = web.copyBackForwardList();
        WebHistoryItem item = list.getItemAtIndex(list.getCurrentIndex() + offset);
        return item == null ? null : item.getUrl();
    }

    private void find(String text, boolean backwards) {
        if (text == null || text.isEmpty()) {
            findText = "";
            web.clearMatches();
            emit("found", "matched", 0);
        } else if (text.equals(findText)) {
            web.findNext(!backwards);
        } else {
            findText = text;
            web.findAllAsync(text);
        }
    }

    private void zoom(Object factor) {
        if (!(factor instanceof Number number)) return;
        textZoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, Math.round(number.floatValue() * 100)));
        web.getSettings().setTextZoom(textZoom);
    }

    private void download(String url, String userAgent, String disposition, String mime, long length) {
        Page next = pending;
        if (next != null && url.equals(next.url())) endNavigation();
        Page page = top;
        String cookies = null;
        CookieManager manager = profileName == null ? null : Stores.cookies(profileName);
        String host = Uri.parse(url).getHost();
        boolean sameHost = page != null && host != null && host.equalsIgnoreCase(Uri.parse(page.url()).getHost());
        if (manager != null && isWeb(url) && sameHost) cookies = manager.getCookie(url);
        Downloads.Request request = new Downloads.Request(url, userAgent, disposition, mime,
                page == null ? null : page.url(), page == null ? level : page.level(), cookies);
        // Refused non-web downloads fail in Downloads.start without using the tab's budget.
        boolean admitted = !isWeb(url) || admit(downloads, DOWNLOAD_BURST, SystemClock.elapsedRealtime());
        Downloads.start(context.getContext(), admitted, request, sortDownloads, data -> {
            if ("finished".equals(data.get("state"))) {
                data.put("url", withoutCredentials(url));
                data.put("quarantined", false);
            }
            deliver(this, data);
        });
    }

    private static void deliver(TarnWebView from, Map<String, Object> data) {
        TarnWebView target = from.destroyed ? LIVE.stream().findFirst().orElse(null) : from;
        if (target == null) {
            UNSENT.add(data);
        } else {
            target.emitMap("download", data);
        }
    }

    private void blockLocal(String url) {
        block(url, "local_network");
        emit("shielded", "kind", "local_network", "url", url);
    }

    private void block(String url, String reason) {
        refusedUrl = url;
        emit("blocked", "url", url, "reason", reason);
    }

    private void emit(String event, Object... pairs) {
        Map<String, Object> data = new LinkedHashMap<>();
        for (int i = 0; i + 1 < pairs.length; i += 2) data.put((String) pairs[i], pairs[i + 1]);
        emitMap(event, data);
    }

    private void emitMap(String event, Map<String, Object> data) {
        Host.onMain(() -> {
            if (destroyed) return;
            Hooks.log("tab " + tab + " event " + event + " " + data);
            context.emit(event, data);
        });
    }

    private void emitPage(String event, WebView view, String url) {
        emit(event, "url", url, "title", String.valueOf(view.getTitle()),
                "canGoBack", view.canGoBack(), "canGoForward", view.canGoForward());
    }

    private static String withoutCredentials(String url) {
        Uri uri = Uri.parse(url);
        String user = uri.getEncodedUserInfo();
        return user == null ? url : url.replaceFirst(java.util.regex.Pattern.quote(user + "@"), "");
    }

    private static boolean sameSchemeAndPort(String a, String b) {
        Uri first = Uri.parse(a);
        Uri second = Uri.parse(b);
        return Objects.equals(first.getScheme(), second.getScheme()) && first.getPort() == second.getPort();
    }

    private static String origin(String url) {
        if (url == null) return "";
        Uri uri = Uri.parse(url);
        if (uri.getScheme() == null || uri.getHost() == null) return "";
        return uri.getScheme() + "://" + uri.getHost() + (uri.getPort() == -1 ? "" : ":" + uri.getPort());
    }

    private void prompt(Prompts.Answer answer, Map<String, Object> data) {
        if (prompts.full()) {
            answer.answer(false, Map.of());
            return;
        }
        Map<String, Object> event = new LinkedHashMap<>();
        event.put("id", prompts.add(answer));
        event.putAll(data);
        emitMap("prompt", event);
    }

    private int kind(WebResourceRequest request) {
        if (request.isForMainFrame()) return Native.MAIN_FRAME;
        String accept = request.getRequestHeaders().getOrDefault("Accept", "");
        String path = request.getUrl().getPath() == null ? "" : request.getUrl().getPath();
        if (accept.startsWith("text/html")) return Native.SUB_FRAME;
        if (accept.startsWith("image/")) return Native.IMAGE;
        if (accept.startsWith("text/css")) return Native.STYLE_SHEET;
        if (path.endsWith(".js") || path.endsWith(".mjs")) return Native.SCRIPT;
        if (path.endsWith(".woff2") || path.endsWith(".woff") || path.endsWith(".ttf")) return Native.FONT;
        return Native.OTHER;
    }

    private static WebResourceResponse refused() {
        return new WebResourceResponse("text/plain", "utf-8", 403, "Blocked",
                Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
    }

    private final class Client extends WebViewClient {
        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            if (!request.isForMainFrame()) return false;
            return !pageNavigation(request.getUrl().toString(), request.isRedirect());
        }

        @Override
        public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
            String target = request.getUrl().toString();
            if (!isWeb(target)) return null;
            int kind = kind(request);
            if (kind == Native.MAIN_FRAME) {
                synchronized (navigation) {
                    return mainFrame(target);
                }
            }
            Page next = pending;
            Page current = top;
            long start = System.nanoTime();
            Page judge = current != null ? current : next;
            int verdict = Native.verdict(target, judge == null ? null : judge.url(), kind,
                    judge == null ? level : judge.level());
            if (next != null && current != null && !next.url().equals(current.url())) {
                verdict = Native.stricter(verdict, Native.verdict(target, next.url(), kind, next.level()));
            }
            Hooks.log("verdict=" + verdict + " us=" + (System.nanoTime() - start) / 1000 + " kind=" + kind
                    + " url=" + target + " top=" + (current == null ? null : current.url())
                    + " pending=" + (next == null ? null : next.url()));
            if (verdict == Native.VERDICT_ALLOW) return null;
            emit("blocked", "url", target, "reason", verdict == Native.VERDICT_BLOCK ? "shield" : "https_only");
            return refused();
        }

        private WebResourceResponse mainFrame(String target) {
            Page next = pending;
            String reason = null;
            boolean expected = next != null && target.equals(next.url());
            int site = levelFor(target);
            int pageLevel = expected ? next.level() : Math.max(site, viewLevel);
            int verdict = Native.verdict(target, target, Native.MAIN_FRAME, pageLevel);
            boolean userStarted = expected && shellNavigation;
            String from = expected ? next.from() : origin();
            if (verdict != Native.VERDICT_BLOCK && Native.intoPrivateNetwork(target, from, userStarted, pageLevel)) {
                verdict = Native.VERDICT_BLOCK;
                reason = "local_network";
                emit("shielded", "kind", "local_network", "url", target);
            }
            if (verdict == Native.VERDICT_ALLOW && !expected && javaScriptOn
                    && Native.levelAction(target, site, viewLevel, false) == Native.CONTINUE_NO_JS) {
                verdict = Native.VERDICT_BLOCK;
                reason = "rebuild";
            }
            if (verdict == Native.VERDICT_ALLOW && !expected) {
                pending = new Page(target, pageLevel, from);
                shellNavigation = false;
            }
            Hooks.log("verdict=" + verdict + " main frame url=" + target + " pending=" + (next == null ? null : next.url()));
            if (verdict == Native.VERDICT_ALLOW) return null;
            if (reason == null) reason = verdict == Native.VERDICT_BLOCK ? "shield" : "https_only";
            block(target, reason);
            return refused();
        }

        @Override
        public void onPageStarted(WebView view, String url, Bitmap favicon) {
            Hooks.log("started " + url);
        }

        @Override
        public void onPageCommitVisible(WebView view, String url) {
            Page current = top;
            if (pending == null && current != null && current.url().equals(url)) return;
            commit(view, url);
        }

        private void commit(WebView view, String url) {
            if (url != null && url.equals(refusedUrl)) return;
            synchronized (navigation) {
                Page next = pending;
                top = new Page(url, next != null ? next.level() : viewLevel, null);
                pending = null;
                refusedUrl = null;
                shellNavigation = false;
            }
            upgraded.clear();
            authAttempts.clear();
            findCache = null;
            prompts.newPage();
            endPick();
            emitPage("committed", view, url);
        }

        @Override
        public void doUpdateVisitedHistory(WebView view, String url, boolean isReload) {
            Page next = pending;
            if (next != null && next.url().equals(url)) {
                commit(view, url);
                return;
            }
            Page current = top;
            if (next == null && current != null && url != null && !url.equals(current.url())
                    && !url.equals(refusedUrl)) {
                top = new Page(url, current.level(), null);
                emitPage("committed", view, url);
            }
        }

        @Override
        public void onPageFinished(WebView view, String url) {
            if (url != null && url.equals(refusedUrl)) return;
            emitPage("finished", view, url);
        }

        @Override
        public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
            String message = String.valueOf(error.getDescription());
            if (request.isForMainFrame()) endNavigation();
            if (request.isForMainFrame() && !message.contains("ERR_ABORTED")) {
                emit("failed", "url", request.getUrl().toString(), "message", message);
            }
        }

        @Override
        public void onReceivedSslError(WebView view, SslErrorHandler handler, SslError error) {
            handler.cancel();
            endNavigation();
            emit("failed", "url", error.getUrl(), "message", "The site's certificate is not valid");
        }

        @Override
        public void onReceivedHttpAuthRequest(WebView view, HttpAuthHandler handler, String host, String realm) {
            Page page = pending != null ? pending : top;
            Page shown = top;
            if (page != null && shown != null && !sameSchemeAndPort(page.url(), shown.url())) {
                handler.cancel();
                return;
            }
            Uri pageUri = page == null ? null : Uri.parse(page.url());
            String samePort = pageUri == null || pageUri.getPort() == -1 ? "" : ":" + pageUri.getPort();
            String challenge = pageUri == null ? "" : pageUri.getScheme() + "://" + host + samePort + "/";
            String origin = origin(challenge);
            int failures = authAttempts.getOrDefault(origin, 0);
            if (page == null || !Native.mayAskCredentials(challenge, page.url(), failures)) {
                handler.cancel();
                return;
            }
            authAttempts.put(origin, failures + 1);
            Map<String, Object> data = new LinkedHashMap<>();
            data.put("kind", "credentials");
            data.put("origin", origin);
            data.put("realm", realm == null ? "" : realm);
            data.put("insecure", !challenge.startsWith("https:"));
            prompt((accept, args) -> {
                String user = Prompts.text(args, "user");
                String password = Prompts.text(args, "password");
                if (accept && user != null) {
                    handler.proceed(user, password == null ? "" : password);
                } else {
                    handler.cancel();
                }
            }, data);
        }

        @Override
        public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
            if (view != web) return true;
            Page current = top;
            if (current != null) loadUrl = current.url();
            tearDown();
            appliedSeq = null;
            emit("crashed");
            return true;
        }
    }

    private final class Chrome extends WebChromeClient {
        @Override
        public void onReceivedTitle(WebView view, String title) {
            emit("title", "title", title == null ? "" : title);
        }

        @Override
        public boolean onJsAlert(WebView view, String url, String message, JsResult result) {
            return dialog("alert", url, message, null, result, (accept, args) -> result.confirm());
        }

        @Override
        public boolean onJsConfirm(WebView view, String url, String message, JsResult result) {
            return dialog("confirm", url, message, null, result, (accept, args) -> {
                if (accept) {
                    result.confirm();
                } else {
                    result.cancel();
                }
            });
        }

        @Override
        public boolean onJsPrompt(WebView view, String url, String message, String value, JsPromptResult result) {
            return dialog("text", url, message, value, result, (accept, args) -> {
                String text = Prompts.text(args, "text");
                if (accept) {
                    result.confirm(text == null ? "" : text);
                } else {
                    result.cancel();
                }
            });
        }

        @Override
        public boolean onJsBeforeUnload(WebView view, String url, String message, JsResult result) {
            result.confirm();
            return true;
        }

        private boolean dialog(String kind, String url, String message, String value, JsResult result,
                               Prompts.Answer answer) {
            if (jsDialogOpen) {
                result.cancel();
                return true;
            }
            jsDialogOpen = true;
            Map<String, Object> data = new LinkedHashMap<>();
            data.put("kind", kind);
            data.put("origin", origin(url));
            data.put("message", message == null ? "" : message);
            if (value != null) data.put("default", value);
            prompt((accept, args) -> {
                jsDialogOpen = false;
                answer.answer(accept, args);
            }, data);
            return true;
        }

        @Override
        public void onPermissionRequest(PermissionRequest request) {
            List<String> granted = new ArrayList<>();
            for (String resource : request.getResources()) {
                if (PermissionRequest.RESOURCE_VIDEO_CAPTURE.equals(resource)
                        || PermissionRequest.RESOURCE_AUDIO_CAPTURE.equals(resource)) {
                    granted.add(resource);
                }
            }
            if (granted.isEmpty()) {
                request.deny();
                return;
            }
            boolean camera = granted.contains(PermissionRequest.RESOURCE_VIDEO_CAPTURE);
            Map<String, Object> data = new LinkedHashMap<>();
            data.put("kind", camera ? "camera" : "microphone");
            data.put("origin", origin(request.getOrigin().toString()));
            Page page = top;
            if (page != null) data.put("topOrigin", origin(page.url()));
            if (camera && granted.size() > 1) data.put("message", "camera and microphone");
            String[] resources = granted.toArray(new String[0]);
            prompt((accept, args) -> {
                if (accept) {
                    request.grant(resources);
                } else {
                    request.deny();
                }
            }, data);
        }

        @Override
        public boolean onShowFileChooser(WebView view, ValueCallback<Uri[]> callback, FileChooserParams params) {
            return Host.activity instanceof ShellActivity shell && shell.chooseFiles(callback, params);
        }

        @Override
        public void onGeolocationPermissionsShowPrompt(String origin, GeolocationPermissions.Callback callback) {
            Map<String, Object> data = new LinkedHashMap<>();
            data.put("kind", "location");
            data.put("origin", origin(origin));
            Page page = top;
            if (page != null) data.put("topOrigin", origin(page.url()));
            prompt((accept, args) -> callback.invoke(origin, accept, false), data);
        }

        @Override
        public boolean onCreateWindow(WebView view, boolean isDialog, boolean isUserGesture, Message resultMsg) {
            if (!isUserGesture || profileName == null) {
                Page page = top;
                if (!isUserGesture && page != null) emit("shielded", "kind", "popup", "url", page.url());
                Hooks.log("popup without a user gesture refused");
                return false;
            }
            if (!admitPopup(SystemClock.elapsedRealtime())) {
                Page page = top;
                if (page != null) emit("shielded", "kind", "popup", "url", page.url());
                Hooks.log("popup over the burst limit refused");
                return false;
            }
            int hit = view.getHitTestResult().getType();
            boolean link = hit == WebView.HitTestResult.SRC_ANCHOR_TYPE || hit == WebView.HitTestResult.SRC_IMAGE_ANCHOR_TYPE;
            PageView catcher = new PageView(context.getContext(), noPersonalizedLearning);
            if (!profileName.isEmpty()) WebViewCompat.setProfile(catcher, profileName);
            catcher.getSettings().setJavaScriptEnabled(false);
            PopupCatcher popup = new PopupCatcher(catcher, link);
            catcher.setWebViewClient(popup);
            WebView.WebViewTransport transport = (WebView.WebViewTransport) resultMsg.obj;
            transport.setWebView(catcher);
            resultMsg.sendToTarget();
            container.postDelayed(() -> popup.caught(null), POPUP_WAIT_MS);
            return true;
        }
    }

    private final class PopupCatcher extends WebViewClient {
        private final WebView catcher;
        private final boolean link;
        private boolean done;

        PopupCatcher(WebView catcher, boolean link) {
            this.catcher = catcher;
            this.link = link;
        }

        void caught(String url) {
            Host.onMain(() -> {
                if (done) return;
                done = true;
                Host.MAIN.post(() -> {
                    catcher.stopLoading();
                    catcher.destroy();
                });
                Page from = top;
                if (url == null || destroyed || !passesNavPolicy(url, Native.POPUP, from == null ? null : from.url())) return;
                if (isWeb(url) && Native.intoPrivateNetwork(url, from == null ? null : from.url(), false,
                        Math.max(levelFor(url), viewLevel))) {
                    refusedUrl = url;
                    emit("blocked", "url", url, "reason", "shield", "popup", true);
                    return;
                }
                emit("popup", "url", url, "link", link);
            });
        }

        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            caught(request.getUrl().toString());
            return true;
        }

        @Override
        public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
            if (request.isForMainFrame()) caught(request.getUrl().toString());
            return refused();
        }

        @Override
        public void onPageStarted(WebView view, String url, Bitmap favicon) {
            if (url != null && !"about:blank".equals(url)) caught(url);
        }
    }
}
