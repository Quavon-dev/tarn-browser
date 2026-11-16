package dev.tarn.android;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.security.KeyPairGenerator;
import java.security.PublicKey;
import java.security.spec.ECGenParameterSpec;
import org.junit.Test;

public class StrongKeysTest {
    private static PublicKey rsa(int bits) throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA");
        generator.initialize(bits);
        return generator.generateKeyPair().getPublic();
    }

    @Test
    public void rsaBelow2048BitsIsRefused() throws Exception {
        assertFalse(StrongKeys.strong(rsa(1024)));
        assertTrue(StrongKeys.strong(rsa(2048)));
    }

    @Test
    public void p256IsAccepted() throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance("EC");
        generator.initialize(new ECGenParameterSpec("secp256r1"));
        assertTrue(StrongKeys.strong(generator.generateKeyPair().getPublic()));
    }

    @Test
    public void aWeakKeyAnywhereInTheChainIsRefused() throws Exception {
        StrongKeys.checkKeys(rsa(2048), rsa(4096), rsa(2048));
        assertThrows(java.security.cert.CertificateException.class, () -> StrongKeys.checkKeys(rsa(2048), rsa(1024), rsa(2048)));
    }

    @Test
    public void otherAlgorithmsAreRefused() throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance("DSA");
        generator.initialize(2048);
        assertFalse(StrongKeys.strong(generator.generateKeyPair().getPublic()));
    }
}
