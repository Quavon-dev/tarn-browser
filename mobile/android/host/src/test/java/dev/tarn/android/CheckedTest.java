package dev.tarn.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import java.math.BigInteger;
import java.net.Socket;
import java.security.KeyFactory;
import java.security.PublicKey;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.security.spec.RSAPublicKeySpec;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLEngine;
import javax.net.ssl.X509ExtendedTrustManager;
import org.junit.Test;

public class CheckedTest {
    private static final X509Certificate[] SENT = new X509Certificate[0];

    private static PublicKey rsa(int bits) throws Exception {
        BigInteger modulus = BigInteger.ONE.shiftLeft(bits - 1).setBit(0);
        return KeyFactory.getInstance("RSA").generatePublic(new RSAPublicKeySpec(modulus, BigInteger.valueOf(65537)));
    }

    private static final class Platform extends X509ExtendedTrustManager {
        private final boolean accept;

        Platform(boolean accept) {
            this.accept = accept;
        }

        private void decide() throws CertificateException {
            if (!accept) throw new CertificateException("platform refused");
        }

        @Override public void checkServerTrusted(X509Certificate[] c, String a, Socket s) throws CertificateException { decide(); }
        @Override public void checkServerTrusted(X509Certificate[] c, String a, SSLEngine e) throws CertificateException { decide(); }
        @Override public void checkServerTrusted(X509Certificate[] c, String a) throws CertificateException { decide(); }
        @Override public void checkClientTrusted(X509Certificate[] c, String a, Socket s) {}
        @Override public void checkClientTrusted(X509Certificate[] c, String a, SSLEngine e) {}
        @Override public void checkClientTrusted(X509Certificate[] c, String a) {}
        @Override public X509Certificate[] getAcceptedIssuers() { return SENT; }
    }

    private static final StrongKeys.KeyPolicy POLICY = (kind, bits) -> kind == StrongKeys.RSA && bits >= 2048;

    private static SSLEngine engine(String host) throws Exception {
        return host == null ? SSLContext.getDefault().createSSLEngine() : SSLContext.getDefault().createSSLEngine(host, 443);
    }

    @Test
    public void aWeakKeyOnTheVerifiedPathIsRefused() throws Exception {
        List<PublicKey> verified = Arrays.asList(rsa(2048), rsa(1024));
        StrongKeys.Checked checked = new StrongKeys.Checked(new Platform(true), (chain, auth, host) -> verified, POLICY);
        assertThrows(CertificateException.class, () -> checked.checkServerTrusted(SENT, "RSA", engine("example.com")));
    }

    @Test
    public void aStrongVerifiedPathPassesAndIsAskedForTheRightHost() throws Exception {
        List<String> hosts = new ArrayList<>();
        StrongKeys.Checked checked = new StrongKeys.Checked(new Platform(true), (chain, auth, host) -> {
            hosts.add(host);
            return Arrays.asList(rsa2048(), rsa2048());
        }, POLICY);
        checked.checkServerTrusted(SENT, "RSA", engine("example.com"));
        assertEquals(Arrays.asList("example.com"), hosts);
    }

    @Test
    public void thePlatformsRefusalWins() throws Exception {
        StrongKeys.Checked checked = new StrongKeys.Checked(new Platform(false), (chain, auth, host) -> Arrays.asList(rsa2048()), POLICY);
        assertThrows(CertificateException.class, () -> checked.checkServerTrusted(SENT, "RSA", engine("example.com")));
    }

    @Test
    public void noHostNameIsRefused() throws Exception {
        StrongKeys.Checked checked = new StrongKeys.Checked(new Platform(true), (chain, auth, host) -> Arrays.asList(rsa2048()), POLICY);
        assertThrows(CertificateException.class, () -> checked.checkServerTrusted(SENT, "RSA", engine(null)));
        assertThrows(CertificateException.class, () -> checked.checkServerTrusted(SENT, "RSA"));
    }

    private static PublicKey rsa2048() throws CertificateException {
        try {
            return rsa(2048);
        } catch (Exception e) {
            throw new CertificateException(e);
        }
    }
}
