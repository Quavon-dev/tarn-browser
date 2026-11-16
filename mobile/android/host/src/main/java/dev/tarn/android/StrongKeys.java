package dev.tarn.android;

import android.net.http.X509TrustManagerExtensions;
import java.io.IOException;
import java.net.Socket;
import java.net.URLConnection;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.security.PublicKey;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.List;
import java.security.interfaces.ECPublicKey;
import java.security.interfaces.RSAPublicKey;
import javax.net.ssl.HttpsURLConnection;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLEngine;
import javax.net.ssl.SSLSession;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.SSLSocketFactory;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509ExtendedTrustManager;

final class StrongKeys {
    static final int MIN_RSA_BITS = 2048;
    static final int MIN_EC_BITS = 224;

    private static SSLSocketFactory factory;

    private StrongKeys() {}

    static void apply(URLConnection connection) throws IOException {
        if (!(connection instanceof HttpsURLConnection)) return;
        try {
            ((HttpsURLConnection) connection).setSSLSocketFactory(factory());
        } catch (GeneralSecurityException e) {
            throw new IOException("TLS is not available", e);
        }
    }

    static boolean strong(PublicKey key) {
        if (key instanceof RSAPublicKey) return ((RSAPublicKey) key).getModulus().bitLength() >= MIN_RSA_BITS;
        if (key instanceof ECPublicKey) return ((ECPublicKey) key).getParams().getCurve().getField().getFieldSize() >= MIN_EC_BITS;
        return "Ed25519".equals(key.getAlgorithm()) || "EdDSA".equals(key.getAlgorithm());
    }

    static void checkKeys(PublicKey... keys) throws CertificateException {
        for (PublicKey key : keys) {
            if (!strong(key)) throw new CertificateException("a certificate of the chain has a key below the minimum");
        }
    }

    private static void check(List<X509Certificate> verified) throws CertificateException {
        PublicKey[] keys = new PublicKey[verified.size()];
        for (int i = 0; i < keys.length; i++) keys[i] = verified.get(i).getPublicKey();
        checkKeys(keys);
    }

    private static synchronized SSLSocketFactory factory() throws GeneralSecurityException {
        if (factory == null) {
            TrustManagerFactory trust = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
            trust.init((KeyStore) null);
            X509ExtendedTrustManager platform = null;
            for (TrustManager manager : trust.getTrustManagers()) {
                if (manager instanceof X509ExtendedTrustManager) platform = (X509ExtendedTrustManager) manager;
            }
            if (platform == null) throw new GeneralSecurityException("no platform trust manager");
            SSLContext context = SSLContext.getInstance("TLS");
            context.init(null, new TrustManager[] {new Checked(platform)}, null);
            factory = context.getSocketFactory();
        }
        return factory;
    }

    private static final class Checked extends X509ExtendedTrustManager {
        private final X509ExtendedTrustManager platform;
        private final X509TrustManagerExtensions extensions;

        Checked(X509ExtendedTrustManager platform) {
            this.platform = platform;
            this.extensions = new X509TrustManagerExtensions(platform);
        }

        private void verify(X509Certificate[] chain, String authType, String host) throws CertificateException {
            if (host == null) throw new CertificateException("no host name for the server's certificate");
            check(extensions.checkServerTrusted(chain, authType, host));
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType, Socket socket) throws CertificateException {
            platform.checkServerTrusted(chain, authType, socket);
            SSLSession session = socket instanceof SSLSocket ? ((SSLSocket) socket).getHandshakeSession() : null;
            verify(chain, authType, session == null ? null : session.getPeerHost());
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType, SSLEngine engine) throws CertificateException {
            platform.checkServerTrusted(chain, authType, engine);
            verify(chain, authType, engine == null ? null : engine.getPeerHost());
        }

        @Override
        public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
            throw new CertificateException("a server certificate is checked only with its host name");
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType, Socket socket) throws CertificateException {
            platform.checkClientTrusted(chain, authType, socket);
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType, SSLEngine engine) throws CertificateException {
            platform.checkClientTrusted(chain, authType, engine);
        }

        @Override
        public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException {
            platform.checkClientTrusted(chain, authType);
        }

        @Override
        public X509Certificate[] getAcceptedIssuers() {
            return platform.getAcceptedIssuers();
        }
    }
}
