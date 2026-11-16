package dev.tarn.android;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.math.BigInteger;
import java.security.KeyFactory;
import java.security.KeyPairGenerator;
import java.security.PublicKey;
import java.security.spec.DSAPublicKeySpec;
import java.security.spec.ECGenParameterSpec;
import java.security.spec.RSAPublicKeySpec;
import org.junit.Test;

public class StrongKeysTest {
    private static PublicKey rsa(int bits) throws Exception {
        BigInteger modulus = BigInteger.ONE.shiftLeft(bits - 1).setBit(0);
        return KeyFactory.getInstance("RSA").generatePublic(new RSAPublicKeySpec(modulus, BigInteger.valueOf(65537)));
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
        BigInteger big = BigInteger.ONE.shiftLeft(2047).setBit(0);
        assertFalse(StrongKeys.strong(KeyFactory.getInstance("DSA").generatePublic(new DSAPublicKeySpec(big, big, BigInteger.valueOf(65537), BigInteger.TWO))));
    }
}
