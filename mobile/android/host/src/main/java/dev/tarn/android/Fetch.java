package dev.tarn.android;

import java.io.IOException;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.Timer;
import java.util.TimerTask;

final class Fetch {
    private static final int CHUNK = 16 * 1024;

    private Fetch() {}

    static Object[] get(String address, long max, String agent, int stallMs, int totalMs) {
        HttpURLConnection connection = null;
        Timer timer = new Timer(true);
        try {
            URL url = new URL(address);
            if (!"https".equals(url.getProtocol())) return new Object[] {"only HTTPS is allowed"};
            connection = (HttpURLConnection) url.openConnection(Hooks.proxy());
            StrongKeys.apply(connection);
            HttpURLConnection watched = connection;
            timer.schedule(new TimerTask() {
                @Override
                public void run() {
                    watched.disconnect();
                }
            }, totalMs);
            connection.setInstanceFollowRedirects(false);
            connection.setUseCaches(false);
            connection.setAllowUserInteraction(false);
            connection.setConnectTimeout(stallMs);
            connection.setReadTimeout(stallMs);
            connection.setRequestProperty("User-Agent", agent);
            int status = connection.getResponseCode();
            if (status < 0) return new Object[] {"The server sent an answer Tarn cannot read."};
            String location = connection.getHeaderField("Location");
            ByteArrayOutputStream body = new ByteArrayOutputStream();
            if (status == 200) {
                try (InputStream in = connection.getInputStream()) {
                    byte[] chunk = new byte[CHUNK];
                    for (int n; (n = in.read(chunk)) != -1; ) {
                        body.write(chunk, 0, n);
                        if (body.size() > max) return new Object[] {"The answer is too large."};
                    }
                }
            }
            return new Object[] {Integer.toString(status), location, body.toByteArray()};
        } catch (IOException | ClassCastException e) {
            return new Object[] {String.valueOf(e.getMessage())};
        } finally {
            timer.cancel();
            if (connection != null) connection.disconnect();
        }
    }

    static Object[] post(String address, String[] headers, byte[] body, int stallMs, int totalMs) {
        HttpURLConnection connection = null;
        Timer timer = new Timer(true);
        try {
            URL url = new URL(address);
            if (!"https".equals(url.getProtocol())) return new Object[] {"only HTTPS is allowed"};
            connection = (HttpURLConnection) url.openConnection(Hooks.proxy());
            StrongKeys.apply(connection);
            HttpURLConnection watched = connection;
            timer.schedule(new TimerTask() {
                @Override
                public void run() {
                    watched.disconnect();
                }
            }, totalMs);
            connection.setInstanceFollowRedirects(false);
            connection.setUseCaches(false);
            connection.setAllowUserInteraction(false);
            connection.setConnectTimeout(stallMs);
            connection.setReadTimeout(stallMs);
            connection.setRequestMethod("POST");
            connection.setDoOutput(true);
            for (int i = 0; i + 1 < headers.length; i += 2) connection.setRequestProperty(headers[i], headers[i + 1]);
            connection.setFixedLengthStreamingMode(body.length);
            try (OutputStream out = connection.getOutputStream()) {
                out.write(body);
            }
            int status = connection.getResponseCode();
            if (status < 0) return new Object[] {"The server sent an answer Tarn cannot read."};
            InputStream in = status < 400 ? connection.getInputStream() : connection.getErrorStream();
            Stream stream = new Stream(connection, in, timer);
            connection = null;
            timer = null;
            return new Object[] {Integer.toString(status), stream};
        } catch (IOException | ClassCastException e) {
            return new Object[] {String.valueOf(e.getMessage())};
        } finally {
            if (timer != null) timer.cancel();
            if (connection != null) connection.disconnect();
        }
    }

    static final class Stream {
        private final HttpURLConnection connection;
        private final InputStream in;
        private final Timer timer;

        Stream(HttpURLConnection connection, InputStream in, Timer timer) {
            this.connection = connection;
            this.in = in;
            this.timer = timer;
        }

        int read(byte[] buffer) throws IOException {
            return in == null ? -1 : in.read(buffer);
        }

        void close() {
            timer.cancel();
            connection.disconnect();
        }
    }
}
