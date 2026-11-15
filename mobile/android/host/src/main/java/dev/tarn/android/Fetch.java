package dev.tarn.android;

import java.io.IOException;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
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
}
