package dev.tarn.android;

final class Native {
    static {
        System.loadLibrary("nsh");
    }

    static final int TYPED = 0;
    static final int LINK = 1;
    static final int POPUP = 2;
    static final int FILE_OPEN = 3;

    static final int ALLOW = 0;
    static final int PROMPT = 1;
    static final int BLOCK = 2;

    static final int REBUILD = 0;
    static final int CONTINUE_JS = 1;
    static final int CONTINUE_NO_JS = 2;
    static final int BLOCK_INSECURE = 3;

    static final int VERDICT_ALLOW = 0;
    static final int VERDICT_BLOCK = 1;
    static final int VERDICT_MAKE_HTTPS = 2;

    static final int MAIN_FRAME = 0;
    static final int SUB_FRAME = 1;
    static final int IMAGE = 2;
    static final int STYLE_SHEET = 3;
    static final int SCRIPT = 4;
    static final int FONT = 5;
    static final int MEDIA = 6;
    static final int FETCH = 7;
    static final int OTHER = 10;

    static final String[] LEVELS = {"standard", "strict", "lockdown"};
    static final int STANDARD = 0;
    static final int LOCKDOWN = 2;

    private Native() {}

    static int level(Object name) {
        for (int i = 0; i < LEVELS.length; i++) {
            if (LEVELS[i].equals(name)) return i;
        }
        return LOCKDOWN;
    }

    static int stricter(int a, int b) {
        if (a == VERDICT_BLOCK || b == VERDICT_BLOCK) return VERDICT_BLOCK;
        return Math.max(a, b);
    }

    static native boolean start(Class<?> host, String dataDir, String privateSession);

    static native int warm();

    static native boolean strongKey(int kind, int bits);

    static native int decide(String url, int source);

    static native int levelAction(String url, int site, int view, boolean fromShell);

    static native boolean intoPrivateNetwork(String target, String from, boolean userStarted, int level);

    static native int verdict(String url, String top, int kind, int level);

    static native String downloadName(String suggested);

    static native String storeInstall(String url);

    static native boolean mayAskCredentials(String challenge, String page, int failures);

    static native int popupBurst();

    static native int downloadBurst();

    static native String readerScript(String page, boolean probe);

    static native String pageTextScript(String page, int maxChars);

    static native String iconLinksScript(String page);

    static native String fetchIconScript(String page, String[] urls);

    static native String picker();

    static native String pickRect();

    static native String insertTextScript(String text);

    static native boolean inserted(String answer);

    static native String selectedTextScript();

    static native String selectionAnswer(String answer);

    static native String mediaAnswer(String answer);

    static native String mediaStateScript();

    static native String mediaCommandScript(String action, double seconds);

    static native double[] snapshotArea(boolean fullPage, double x, double y, double width, double height, double scale);

    static native String findTextScript(String page);

    static native String[] findSnippets(String text, String query);

    static native String siteStylesScript(String host, String css, String[] zaps, String theme);

    static native String themeStylesScript(String theme);

    static native String[] themeOrigins(String theme);

    static native String downloadFolder(String name);

    static native long burstWindowMs();

    static native void setWorldsSupported(boolean supported);

    static native String[] extensionWorlds(String store);

    static native String extensionLoader();

    static native String extensionPageScript(String host, String bootstrap);

    static native int extensionVerdict(String store, String url, String top, int level, String next, int nextLevel, int kind);

    static native Object[] extensionResource(String store, String url);
}
