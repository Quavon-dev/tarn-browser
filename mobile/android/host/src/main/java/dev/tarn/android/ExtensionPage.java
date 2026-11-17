package dev.tarn.android;

import android.net.Uri;
import android.net.http.SslError;
import android.os.Message;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.GeolocationPermissions;
import android.webkit.JsPromptResult;
import android.webkit.JsResult;
import android.webkit.PermissionRequest;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.SslErrorHandler;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;

import androidx.webkit.JavaScriptReplyProxy;
import androidx.webkit.ScriptHandler;
import androidx.webkit.WebMessageCompat;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;

import com.longbridge.nsh.NshComponent;
import com.longbridge.nsh.NshComponentContext;

import java.io.ByteArrayInputStream;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;
import java.util.Set;

final class ExtensionPage implements NshComponent {
    private static final String SCHEME = "tarn-extension";
    private static final String WORLD_PREFIX = "tarn-ext-";

    private final NshComponentContext context;
    private final FrameLayout container;
    private PageView web;
    private ScriptHandler script;
    private JavaScriptReplyProxy page;
    private String profileName;
    private String built;
    private boolean applyPosted;
    private boolean destroyed;

    private String store;
    private String world;
    private String url;
    private String bootstrap;
    private volatile String servedStore;
    private volatile String servedHost;
    private volatile int level = Native.LOCKDOWN;

    ExtensionPage(NshComponentContext context) {
        this.context = context;
        container = new FrameLayout(context.getContext());
        container.setContentDescription("Extension");
    }

    @Override
    public View view() {
        return container;
    }

    @Override
    public void setProp(String name, Object value) {
        String text = value instanceof String s ? s : null;
        switch (name) {
            case "profile" -> store = text;
            case "world" -> world = text;
            case "url" -> url = text;
            case "bootstrap" -> bootstrap = text;
            case "level" -> level = Native.level(value);
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
        if (!"extensionPost".equals(name) || !(args instanceof Map<?, ?> map)) return;
        String json = Prompts.text(map, "json");
        if (page != null && json != null && Objects.equals(world, Prompts.text(map, "world"))) page.postMessage(json);
    }

    @Override
    public void destroy() {
        destroyed = true;
        tearDown();
    }

    private static String host(String url) {
        Uri uri = url == null ? null : Uri.parse(url);
        return uri != null && SCHEME.equals(uri.getScheme()) && uri.getHost() != null && !uri.getHost().isEmpty() ? uri.getHost() : null;
    }

    private void apply() {
        applyPosted = false;
        if (destroyed) return;
        String key = store + "|" + world + "|" + url + "|" + level + "|" + Objects.hashCode(bootstrap);
        if (key.equals(built)) return;
        tearDown();
        built = key;
        String own = host(url);
        String name = Stores.profileName(store);
        boolean supported = WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)
                && WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT);
        if (own == null || name == null || bootstrap == null || world == null || !world.startsWith(WORLD_PREFIX)
                || level == Native.LOCKDOWN || !Stores.persistent(store) || !supported) {
            Hooks.log("extension page not opened");
            return;
        }
        PageView view = new PageView(context.getContext(), false);
        Stores.attach(view, name);
        configure(view);
        Set<String> anywhere = Set.of("*");
        String setup = Native.extensionPageScript(own, bootstrap);
        try {
            if (setup == null) throw new IllegalArgumentException("no page script");
            script = WebViewCompat.addDocumentStartJavaScript(view, setup, anywhere);
            WebViewCompat.addWebMessageListener(view, Extensions.PORT, anywhere, this::received);
        } catch (IllegalArgumentException e) {
            Hooks.log("extension page bridge refused: " + e.getMessage());
            if (script != null) script.remove();
            script = null;
            Stores.detach(name);
            view.destroy();
            return;
        }
        container.addView(view, new FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        web = view;
        profileName = name;
        servedStore = store;
        servedHost = own;
        view.loadUrl(url);
    }

    private void configure(PageView view) {
        WebView.setWebContentsDebuggingEnabled(false);
        WebSettings settings = view.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setJavaScriptCanOpenWindowsAutomatically(false);
        settings.setSupportMultipleWindows(false);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setGeolocationEnabled(false);
        view.setWebViewClient(new Client());
        view.setWebChromeClient(new Chrome());
    }

    private void tearDown() {
        page = null;
        servedStore = null;
        servedHost = null;
        if (script != null) script.remove();
        script = null;
        if (web == null) return;
        PageView old = web;
        web = null;
        WebViewCompat.removeWebMessageListener(old, Extensions.PORT);
        container.removeView(old);
        old.destroy();
        Stores.detach(profileName);
        profileName = null;
    }

    private void received(WebView view, WebMessageCompat message, Uri origin, boolean main, JavaScriptReplyProxy reply) {
        if (view != web || !main || message.getType() != WebMessageCompat.TYPE_STRING || !Objects.equals(host(view.getUrl()), servedHost)) return;
        String json = message.getData();
        if (json == null || json.length() > Extensions.MAX_MESSAGE_CHARS || !json.startsWith("{")) return;
        page = reply;
        Map<String, Object> data = new LinkedHashMap<>();
        data.put("world", world);
        data.put("frame", 0L);
        data.put("json", json);
        Host.onMain(() -> {
            if (!destroyed) context.emit("extensionMessage", data);
        });
    }

    private static WebResourceResponse response(int status, String mime, String csp, byte[] body) {
        Map<String, String> headers = new LinkedHashMap<>();
        if (csp != null) headers.put("Content-Security-Policy", csp);
        headers.put("X-Content-Type-Options", "nosniff");
        String reason = switch (status) {
            case 200 -> "OK";
            case 400 -> "Bad Request";
            case 404 -> "Not Found";
            default -> "Forbidden";
        };
        boolean text = mime.startsWith("text/") || mime.endsWith("json") || mime.endsWith("javascript") || mime.endsWith("svg+xml");
        return new WebResourceResponse(mime, text ? "utf-8" : null, status >= 100 && status < 600 && (status < 300 || status >= 400) ? status : 403,
                reason, headers, new ByteArrayInputStream(body));
    }

    private static WebResourceResponse refused() {
        return response(403, "text/plain", "default-src 'none'", new byte[0]);
    }

    private final class Client extends WebViewClient {
        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            if (!request.isForMainFrame() && "about".equals(request.getUrl().getScheme())) return false;
            return !Objects.equals(host(request.getUrl().toString()), servedHost);
        }

        @Override
        public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
            String target = request.getUrl().toString();
            String scheme = request.getUrl().getScheme();
            if (SCHEME.equals(scheme)) {
                String own = servedHost;
                String rules = servedStore;
                if (own == null || rules == null || !own.equals(request.getUrl().getHost())) return refused();
                Object[] found = Native.extensionResource(rules, target);
                if (found == null || found.length != 4 || !(found[0] instanceof String status) || !(found[1] instanceof String mime)
                        || !(found[3] instanceof byte[] body)) {
                    return refused();
                }
                int code;
                try {
                    code = Integer.parseInt(status);
                } catch (NumberFormatException e) {
                    return refused();
                }
                return response(code, mime, found[2] instanceof String csp ? csp : null, body);
            }
            if (!"http".equals(scheme) && !"https".equals(scheme)) return null;
            int kind = TarnWebView.kind(request);
            if (kind == Native.MAIN_FRAME || kind == Native.SUB_FRAME) return refused();
            if (Native.verdict(target, null, kind, level) == Native.VERDICT_ALLOW) return null;
            Hooks.log("extension page request refused " + target);
            return refused();
        }

        @Override
        public void onReceivedSslError(WebView view, SslErrorHandler handler, SslError error) {
            handler.cancel();
        }

        @Override
        public boolean onRenderProcessGone(WebView view, RenderProcessGoneDetail detail) {
            if (view == web) {
                built = null;
                tearDown();
            }
            return true;
        }
    }

    private static final class Chrome extends WebChromeClient {
        @Override
        public boolean onJsAlert(WebView view, String url, String message, JsResult result) {
            result.cancel();
            return true;
        }

        @Override
        public boolean onJsConfirm(WebView view, String url, String message, JsResult result) {
            result.cancel();
            return true;
        }

        @Override
        public boolean onJsPrompt(WebView view, String url, String message, String value, JsPromptResult result) {
            result.cancel();
            return true;
        }

        @Override
        public boolean onJsBeforeUnload(WebView view, String url, String message, JsResult result) {
            result.confirm();
            return true;
        }

        @Override
        public void onPermissionRequest(PermissionRequest request) {
            request.deny();
        }

        @Override
        public void onGeolocationPermissionsShowPrompt(String origin, GeolocationPermissions.Callback callback) {
            callback.invoke(origin, false, false);
        }

        @Override
        public boolean onShowFileChooser(WebView view, ValueCallback<Uri[]> callback, FileChooserParams params) {
            return false;
        }

        @Override
        public boolean onCreateWindow(WebView view, boolean isDialog, boolean isUserGesture, Message resultMsg) {
            return false;
        }
    }
}
