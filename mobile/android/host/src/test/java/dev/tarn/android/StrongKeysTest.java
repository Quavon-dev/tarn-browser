package dev.tarn.android;

import static org.junit.Assert.assertArrayEquals;

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
    public void rsaKeysReportTheirModulusLength() throws Exception {
        assertArrayEquals(new int[] {StrongKeys.RSA, 1024}, StrongKeys.describe(rsa(1024)));
        assertArrayEquals(new int[] {StrongKeys.RSA, 2048}, StrongKeys.describe(rsa(2048)));
    }

    @Test
    public void ecKeysReportTheirFieldSize() throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance("EC");
        generator.initialize(new ECGenParameterSpec("secp256r1"));
        assertArrayEquals(new int[] {StrongKeys.EC, 256}, StrongKeys.describe(generator.generateKeyPair().getPublic()));
    }

    @Test
    public void otherKeyTypesReportOther() throws Exception {
        BigInteger big = BigInteger.ONE.shiftLeft(2047).setBit(0);
        PublicKey dsa = KeyFactory.getInstance("DSA").generatePublic(new DSAPublicKeySpec(big, big, BigInteger.valueOf(65537), BigInteger.TWO));
        assertArrayEquals(new int[] {StrongKeys.OTHER, 0}, StrongKeys.describe(dsa));
    }
}
