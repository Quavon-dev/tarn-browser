package dev.tarn.android;

import android.content.Context;
import android.net.Uri;
import android.os.SystemClock;
import android.webkit.URLUtil;

import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;

final class Downloads {
    interface Listener {
        void download(Map<String, Object> data);
    }

    record Request(String url, String userAgent, String disposition, String mime, String top, int level,
                   String cookies) {}

    private static final int MAX_REDIRECTS = 5;
    private static final int TIMEOUT_MS = 30_000;
    private static final int BUFFER = 64 * 1024;
    private static final long PROGRESS_MS = 500;
    private static final ExecutorService WORKERS = Executors.newCachedThreadPool();
    private static final AtomicLong NEXT_ID = new AtomicLong();
    private static final Map<String, Transfer> ACTIVE = new HashMap<>();
    private static final int RANGE_NOT_SATISFIABLE = 416;
    private static final String PART_PREFIX = "download";
    private static final String PART_SUFFIX = ".part";
    private static final String PARTS = "Downloads.parts";

    private Downloads() {}

    static final class Transfer {
        final String id;
        final String name;
        private final Request request;
        private final File dir;
        private final boolean sort;
        private final Listener listener;
        private volatile boolean pausing;
        private volatile boolean cancelled;
        private boolean running;
        private File part;
        private long done;
        private String resumeUrl;
        private String validator;

        private Transfer(Request request, File dir, String name, boolean sort, Listener listener) {
            this.id = "a" + NEXT_ID.incrementAndGet();
            this.request = request;
            this.dir = dir;
            this.name = name;
            this.sort = sort;
            this.listener = listener;
        }

        private void post(Object... pairs) {
            Map<String, Object> data = new LinkedHashMap<>();
            data.put("id", id);
            for (int i = 0; i + 1 < pairs.length; i += 2) data.put((String) pairs[i], pairs[i + 1]);
            Object state = data.get("state");
            boolean ended = "finished".equals(state) || "failed".equals(state);
            Host.MAIN.post(() -> {
                if (ended) ACTIVE.remove(id);
                listener.download(data);
            });
        }

        synchronized void pause() {
            if (running) pausing = true;
        }

        synchronized void resume() {
            if (running) {
                pausing = false;
                return;
            }
            if (cancelled || part == null) return;
            pausing = false;
            running = true;
            post("state", "resumed");
            WORKERS.execute(this::run);
        }

        synchronized void cancel() {
            cancelled = true;
            ACTIVE.remove(id);
            if (!running) deletePart();
        }

        private void deletePart() {
            if (part == null) return;
            try {
                Files.deleteIfExists(part.toPath());
                part = null;
            } catch (IOException ignored) {
                return;
            }
        }

        private void run() {
            try {
                File file = fetch();
                if (file == null) return;
                post("state", "finished", "name", file.getName(), "path", file.getPath());
            } catch (IOException e) {
                synchronized (this) {
                    running = false;
                    deletePart();
                }
                if (!cancelled) post("state", "failed", "name", name, "message", e.getMessage() == null ? "Download failed" : e.getMessage());
            }
        }

        private File fetch() throws IOException {
            String target = resumeUrl != null ? resumeUrl : request.url();
            String cookies = target.equals(request.url()) ? request.cookies() : null;
            for (int hop = 0; hop <= MAX_REDIRECTS; hop++) {
                target = checked(target, request);
                HttpURLConnection connection = (HttpURLConnection) new URL(target).openConnection();
                StrongKeys.apply(connection);
                try {
                    connection.setInstanceFollowRedirects(false);
                    connection.setConnectTimeout(TIMEOUT_MS);
                    connection.setReadTimeout(TIMEOUT_MS);
                    if (request.userAgent() != null) connection.setRequestProperty("User-Agent", request.userAgent());
                    if (cookies != null) connection.setRequestProperty("Cookie", cookies);
                    if (done > 0 && validator != null) {
                        connection.setRequestProperty("Range", "bytes=" + done + "-");
                        connection.setRequestProperty("If-Range", validator);
                    }
                    int status = connection.getResponseCode();
                    if (status == RANGE_NOT_SATISFIABLE && done > 0 && part != null
                            && ("bytes */" + done).equals(connection.getHeaderField("Content-Range"))) {
                        return complete();
                    }
                    if (status >= 300 && status < 400) {
                        String location = connection.getHeaderField("Location");
                        if (location == null) throw new IOException("Redirect without a location");
                        target = new URL(new URL(target), location).toString();
                        cookies = null;
                        continue;
                    }
                    String range = connection.getHeaderField("Content-Range");
                    boolean partial = status == HttpURLConnection.HTTP_PARTIAL && done > 0 && range != null && range.startsWith("bytes " + done + "-");
                    if (status == HttpURLConnection.HTTP_PARTIAL && !partial) throw new IOException("The server sent another part of the file");
                    if (status != HttpURLConnection.HTTP_OK && !partial) throw new IOException("Server answered " + status);
                    if (!partial) {
                        done = 0;
                        String etag = connection.getHeaderField("ETag");
                        validator = etag != null && !etag.startsWith("W/") ? etag : connection.getHeaderField("Last-Modified");
                    }
                    resumeUrl = target;
                    long length = connection.getContentLengthLong();
                    return save(connection, length < 0 ? -1 : length + done, partial);
                } finally {
                    connection.disconnect();
                }
            }
            throw new IOException("Too many redirects");
        }

        private File save(HttpURLConnection connection, long total, boolean append) throws IOException {
            File parts = parts(dir);
            if (!dir.isDirectory() && !dir.mkdirs()) throw new IOException("No download folder");
            if (!parts.isDirectory() && !parts.mkdirs()) throw new IOException("No download folder");
            if (part == null) part = File.createTempFile(PART_PREFIX, PART_SUFFIX, parts);
            StandardOpenOption mode = append ? StandardOpenOption.APPEND : StandardOpenOption.TRUNCATE_EXISTING;
            long sent = 0;
            boolean whole = false;
            try (InputStream in = connection.getInputStream();
                 OutputStream out = Files.newOutputStream(part.toPath(), StandardOpenOption.WRITE, mode)) {
                byte[] buffer = new byte[BUFFER];
                while (!pausing || cancelled) {
                    int n = in.read(buffer);
                    if (cancelled) throw new IOException("Cancelled");
                    if (n < 0) {
                        whole = true;
                        break;
                    }
                    out.write(buffer, 0, n);
                    done += n;
                    long now = SystemClock.elapsedRealtime();
                    if (now - sent >= PROGRESS_MS) {
                        sent = now;
                        if (total > 0) post("state", "progress", "done", done, "total", total);
                        else post("state", "progress", "done", done);
                    }
                }
            }
            if (!whole && total > 0 && done >= total) whole = true;
            if (!whole) {
                synchronized (this) {
                    if (cancelled) throw new IOException("Cancelled");
                    running = false;
                    pausing = false;
                }
                post("state", "paused", "done", done);
                return null;
            }
            return complete();
        }

        private File complete() throws IOException {
            File saved;
            synchronized (this) {
                if (cancelled) throw new IOException("Cancelled");
                saved = part;
            }
            File file = place(saved, sort ? sorted(dir, name) : dir, name);
            synchronized (this) {
                running = false;
                part = null;
                if (cancelled) {
                    Files.deleteIfExists(file.toPath());
                    throw new IOException("Cancelled");
                }
                return file;
            }
        }
    }

    static Transfer find(String id) {
        return id == null ? null : ACTIVE.get(id);
    }

    static void sweep(Context context) {
        File[] parts = parts(directory(context)).listFiles();
        if (parts == null) return;
        for (File part : parts) {
            try {
                Files.deleteIfExists(part.toPath());
            } catch (IOException ignored) {
                continue;
            }
        }
    }

    static File directory(Context context) {
        String session = Host.privateSession;
        File base = session == null ? context.getFilesDir() : PrivateSpace.sessionDir(context, session);
        return new File(base, "Downloads");
    }

    private static File parts(File downloads) {
        return new File(downloads.getParentFile(), PARTS);
    }

    static String name(Request request) {
        return Native.downloadName(URLUtil.guessFileName(request.url(), request.disposition(), request.mime()));
    }

    /** {@code admitted}: the tab is within its download limit ({@code Native.downloadBurst}). */
    static Transfer start(Context context, boolean admitted, Request request, boolean sort, Listener listener) {
        String name = name(request);
        Transfer transfer = new Transfer(request, directory(context), name, sort, listener);
        String scheme = Uri.parse(request.url()).getScheme();
        if (!"http".equals(scheme) && !"https".equals(scheme)) {
            transfer.post("state", "failed", "name", name, "message", "Only web downloads are supported");
            return null;
        }
        if (!admitted) {
            transfer.post("state", "failed", "name", name, "message", "Too many downloads at once.");
            return null;
        }
        transfer.post("state", "started", "name", name, "pausable", true);
        synchronized (transfer) {
            transfer.running = true;
        }
        ACTIVE.put(transfer.id, transfer);
        WORKERS.execute(transfer::run);
        return transfer;
    }

    private static String checked(String target, Request request) throws IOException {
        if (Native.decide(target, Native.LINK) != Native.ALLOW) throw new IOException("Blocked by navigation policy");
        int verdict = Native.verdict(target, request.top(), Native.OTHER, request.level());
        if (verdict == Native.VERDICT_BLOCK) throw new IOException("Blocked by Shield");
        if (verdict == Native.VERDICT_MAKE_HTTPS) return "https" + target.substring("http".length());
        return target;
    }

    private static File sorted(File dir, String name) {
        String folder = Native.downloadFolder(name);
        if (folder == null) return dir;
        File sub = new File(dir, folder);
        if (Files.isDirectory(sub.toPath(), LinkOption.NOFOLLOW_LINKS)) return sub;
        if (Files.exists(sub.toPath(), LinkOption.NOFOLLOW_LINKS)) return dir;
        return sub.mkdir() || Files.isDirectory(sub.toPath(), LinkOption.NOFOLLOW_LINKS) ? sub : dir;
    }

    static File place(File part, File dir, String name) throws IOException {
        synchronized (Downloads.class) {
            File file = unique(dir, name);
            Files.move(part.toPath(), file.toPath(), StandardCopyOption.ATOMIC_MOVE);
            return file;
        }
    }

    private static File unique(File dir, String name) {
        File file = new File(dir, name);
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String ext = dot > 0 ? name.substring(dot) : "";
        for (int i = 2; Files.exists(file.toPath(), LinkOption.NOFOLLOW_LINKS); i++) file = new File(dir, stem + " (" + i + ")" + ext);
        return file;
    }
}
