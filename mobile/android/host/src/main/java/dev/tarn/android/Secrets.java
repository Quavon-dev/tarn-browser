package dev.tarn.android;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.KeyStore;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

final class Secrets {
    private static final String KEYSTORE = "AndroidKeyStore";
    private static final String KEY = "tarn-secrets";
    private static final String PREFS = "tarn-secrets";
    private static final String CIPHER = "AES/GCM/NoPadding";
    private static final int IV_BYTES = 12;
    private static final int TAG_BITS = 128;

    private Secrets() {}

    static String get(Context context, String name) throws GeneralSecurityException {
        String stored = prefs(context).getString(name, null);
        if (stored == null) return null;
        byte[] sealed = Base64.decode(stored, Base64.NO_WRAP);
        if (sealed.length <= IV_BYTES) throw new GeneralSecurityException("the stored secret is damaged");
        Cipher cipher = Cipher.getInstance(CIPHER);
        cipher.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES));
        cipher.updateAAD(name.getBytes(StandardCharsets.UTF_8));
        return new String(cipher.doFinal(sealed, IV_BYTES, sealed.length - IV_BYTES), StandardCharsets.UTF_8);
    }

    static void set(Context context, String name, String value) throws GeneralSecurityException {
        Cipher cipher = Cipher.getInstance(CIPHER);
        cipher.init(Cipher.ENCRYPT_MODE, key());
        cipher.updateAAD(name.getBytes(StandardCharsets.UTF_8));
        byte[] iv = cipher.getIV();
        byte[] body = cipher.doFinal(value.getBytes(StandardCharsets.UTF_8));
        if (iv.length != IV_BYTES) throw new GeneralSecurityException("unexpected IV length");
        byte[] sealed = new byte[iv.length + body.length];
        System.arraycopy(iv, 0, sealed, 0, iv.length);
        System.arraycopy(body, 0, sealed, iv.length, body.length);
        if (!prefs(context).edit().putString(name, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) {
            throw new GeneralSecurityException("the secret could not be saved");
        }
    }

    static void delete(Context context, String name) throws GeneralSecurityException {
        if (!prefs(context).edit().remove(name).commit()) throw new GeneralSecurityException("the secret could not be removed");
    }

    private static SharedPreferences prefs(Context context) {
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    private static synchronized SecretKey key() throws GeneralSecurityException {
        KeyStore store = KeyStore.getInstance(KEYSTORE);
        try {
            store.load(null);
        } catch (IOException e) {
            throw new GeneralSecurityException(e);
        }
        if (store.getEntry(KEY, null) instanceof KeyStore.SecretKeyEntry entry) return entry.getSecretKey();
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE);
        generator.init(new KeyGenParameterSpec.Builder(KEY, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build());
        return generator.generateKey();
    }
}
