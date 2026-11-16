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
import java.util.ArrayList;
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
    static final int RSA = 1;
    static final int EC = 2;
    static final int ED25519 = 3;
    static final int OTHER = 0;

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

    static int[] describe(PublicKey key) {
        if (key instanceof RSAPublicKey) return new int[] {RSA, ((RSAPublicKey) key).getModulus().bitLength()};
        if (key instanceof ECPublicKey) return new int[] {EC, ((ECPublicKey) key).getParams().getCurve().getField().getFieldSize()};
        if ("Ed25519".equals(key.getAlgorithm()) || "EdDSA".equals(key.getAlgorithm())) return new int[] {ED25519, 255};
        return new int[] {OTHER, 0};
    }

    interface VerifiedPath {
        List<PublicKey> keys(X509Certificate[] chain, String authType, String host) throws CertificateException;
    }

    interface KeyPolicy {
        boolean strong(int kind, int bits);
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
            X509TrustManagerExtensions extensions = new X509TrustManagerExtensions(platform);
            VerifiedPath path = (chain, authType, host) -> {
                List<PublicKey> keys = new ArrayList<>();
                for (X509Certificate certificate : extensions.checkServerTrusted(chain, authType, host)) keys.add(certificate.getPublicKey());
                return keys;
            };
            context.init(null, new TrustManager[] {new Checked(platform, path, Native::strongKey)}, null);
            factory = context.getSocketFactory();
        }
        return factory;
    }

    static final class Checked extends X509ExtendedTrustManager {
        private final X509ExtendedTrustManager platform;
        private final VerifiedPath path;
        private final KeyPolicy policy;

        Checked(X509ExtendedTrustManager platform, VerifiedPath path, KeyPolicy policy) {
            this.platform = platform;
            this.path = path;
            this.policy = policy;
        }

        private void verify(X509Certificate[] chain, String authType, String host) throws CertificateException {
            if (host == null) throw new CertificateException("no host name for the server's certificate");
            for (PublicKey key : path.keys(chain, authType, host)) {
                int[] described = describe(key);
                if (!policy.strong(described[0], described[1])) {
                    throw new CertificateException("a certificate of the verified path has a key below the minimum");
                }
            }
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
