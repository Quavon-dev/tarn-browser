package dev.tarn.android;

import android.app.Activity;
import android.app.KeyguardManager;
import android.hardware.biometrics.BiometricManager;
import android.hardware.biometrics.BiometricPrompt;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.speech.tts.TextToSpeech;
import android.speech.tts.Voice;
import android.os.Handler;
import android.os.Looper;

import androidx.core.content.FileProvider;

import java.io.File;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.security.GeneralSecurityException;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

final class Host {
    static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static final long IMPORT_WAIT_S = 5;
    private static final String CARD_DIR = "card";
    static volatile Context app;
    static volatile String privateSession;
    static PrivateActivity privateActivity;
    static Activity activity;
    private static TextToSpeech speech;
    private static String pendingSpeech;
    private static final int SPEECH_PENDING = 0;
    private static final int SPEECH_STARTED = 1;
    private static final int SPEECH_FAILED = 2;
    private static volatile int speechState = SPEECH_STARTED;
    /** Request codes of the confirm-credential screen: this bit plus the low bits of its check's number. */
    static final int CONFIRM_OWNER = 0x1000;
    private static final int CHECK_BITS = 0x0fff;
    private static final int OWNER_ASKING = 0;
    private static final int OWNER_CONFIRMED = 1;
    private static final int OWNER_REFUSED = 2;
    private static int ownerState = OWNER_REFUSED;
    private static int ownerRequest;
    private static int ownerScreen;
    private static CancellationSignal ownerPrompt;

    private Host() {}

    static void wipeStore(String uuid) {
        MAIN.post(() -> Stores.wipe(app, uuid));
    }

    static int importCookies(String store, String[] urls, String[] cookies) {
        if (Looper.myLooper() == Looper.getMainLooper()) return Stores.importCookies(store, urls, cookies);
        AtomicInteger written = new AtomicInteger();
        AtomicBoolean abandoned = new AtomicBoolean();
        CountDownLatch done = new CountDownLatch(1);
        MAIN.post(() -> {
            if (!abandoned.get()) written.set(Stores.importCookies(store, urls, cookies));
            done.countDown();
        });
        try {
            if (done.await(IMPORT_WAIT_S, TimeUnit.SECONDS)) return written.get();
            abandoned.set(true);
            return done.getCount() == 0 ? written.get() : 0;
        } catch (InterruptedException e) {
            abandoned.set(true);
            Thread.currentThread().interrupt();
            return 0;
        }
    }

    static boolean share(String url) {
        if (activity == null) return false;
        MAIN.post(() -> {
            if (activity == null) return;
            Intent send = new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url);
            activity.startActivity(Intent.createChooser(send, null));
        });
        return true;
    }

    static boolean shareFile(String path) {
        if (activity == null || app == null) return false;
        File source = new File(path);
        File parent = source.getParentFile();
        if (parent == null) return false;
        File card = new File(new File(new File(app.getCacheDir(), CARD_DIR), parent.getName()), source.getName());
        Uri uri;
        try {
            Files.createDirectories(card.getParentFile().toPath());
            Files.copy(Paths.get(path), card.toPath(), StandardCopyOption.REPLACE_EXISTING);
            uri = FileProvider.getUriForFile(app, app.getPackageName() + "." + CARD_DIR, card);
        } catch (IOException | RuntimeException e) {
            return false;
        }
        MAIN.post(() -> {
            if (activity == null) return;
            Intent send = new Intent(Intent.ACTION_SEND)
                    .setType(app.getContentResolver().getType(uri))
                    .putExtra(Intent.EXTRA_STREAM, uri)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            send.setClipData(ClipData.newRawUri(null, uri));
            activity.startActivity(Intent.createChooser(send, null));
        });
        return true;
    }

    static void speak(String text) {
        speechState = SPEECH_PENDING;
        MAIN.post(() -> {
            if (app == null) {
                speechState = SPEECH_FAILED;
                return;
            }
            if (speech == null || pendingSpeech != null) {
                boolean starting = speech != null;
                pendingSpeech = text;
                if (starting) return;
                speech = new TextToSpeech(app, status -> {
                    String waiting = pendingSpeech;
                    pendingSpeech = null;
                    if (status != TextToSpeech.SUCCESS) {
                        speech = null;
                        speechState = SPEECH_FAILED;
                    } else if (waiting != null) {
                        say(waiting);
                    }
                });
                return;
            }
            say(text);
        });
    }

    private static void say(String text) {
        speech.stop();
        Bundle params = new Bundle();
        params.putString(TextToSpeech.Engine.KEY_FEATURE_NETWORK_SYNTHESIS, "false");
        params.putString(TextToSpeech.Engine.KEY_FEATURE_EMBEDDED_SYNTHESIS, "true");
        Voice local = speech.getVoices() == null ? null : speech.getVoices().stream()
                .filter(v -> !v.isNetworkConnectionRequired() && !v.getFeatures().contains(TextToSpeech.Engine.KEY_FEATURE_NETWORK_SYNTHESIS))
                .filter(v -> speech.getVoice() == null || v.getLocale().equals(speech.getVoice().getLocale()))
                .findFirst().orElse(null);
        if (local == null) {
            speechState = SPEECH_FAILED;
            return;
        }
        speechState = SPEECH_STARTED;
        speech.setVoice(local);
        int max = TextToSpeech.getMaxSpeechInputLength();
        for (int at = 0; at < text.length(); ) {
            int end = Math.min(text.length(), at + max);
            if (end < text.length() && Character.isHighSurrogate(text.charAt(end - 1))) end--;
            speech.speak(text.substring(at, end), at == 0 ? TextToSpeech.QUEUE_FLUSH : TextToSpeech.QUEUE_ADD, params, "tarn-" + at);
            at = end;
        }
    }

    static int speechState() {
        return speechState;
    }

    static void stopSpeaking(String ignored) {
        MAIN.post(() -> {
            pendingSpeech = null;
            if (speech != null) speech.stop();
        });
    }

    /** Starts owner check number n (above 0) and returns it, or 0 while another runs. */
    static synchronized int confirmOwner(String reason) {
        if (activity == null || ownerState == OWNER_ASKING) return 0;
        int request = ++ownerRequest;
        ownerState = OWNER_ASKING;
        MAIN.post(() -> askOwner(reason, request));
        return request;
    }

    @SuppressWarnings("deprecation")
    private static void askOwner(String reason, int request) {
        Activity shown = activity;
        String title = "Confirm it is you";
        String description = "Tarn is trying to " + reason + ".";
        try {
            if (shown == null) {
                ownerAnswered(request, false);
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                BiometricPrompt.CryptoObject crypto = new BiometricPrompt.CryptoObject(ownerCipher());
                CancellationSignal cancel = new CancellationSignal();
                ownerPrompt = cancel;
                new BiometricPrompt.Builder(shown).setTitle(title).setDescription(description)
                        .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG | BiometricManager.Authenticators.DEVICE_CREDENTIAL)
                        .build()
                        .authenticate(crypto, cancel, shown.getMainExecutor(), new BiometricPrompt.AuthenticationCallback() {
                            @Override
                            public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                                ownerAnswered(request, keyUnlocked(result.getCryptoObject()));
                            }

                            @Override
                            public void onAuthenticationError(int code, CharSequence message) {
                                ownerAnswered(request, false);
                            }
                        });
            } else {
                KeyguardManager keyguard = shown.getSystemService(KeyguardManager.class);
                Intent ask = keyguard == null ? null : keyguard.createConfirmDeviceCredentialIntent(title, description);
                if (ask == null) {
                    ownerAnswered(request, false);
                } else {
                    ownerScreen = request;
                    shown.startActivityForResult(ask, CONFIRM_OWNER | (request & CHECK_BITS));
                }
            }
        } catch (RuntimeException | GeneralSecurityException e) {
            ownerAnswered(request, false);
        }
    }

    /** A fresh Keystore key that encrypts only right after the owner confirmed, so a success the system did not send cannot use it. */
    private static Cipher ownerCipher() throws GeneralSecurityException {
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder("tarn-owner-check", KeyProperties.PURPOSE_ENCRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setUserAuthenticationRequired(true)
                .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG | KeyProperties.AUTH_DEVICE_CREDENTIAL)
                .build());
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, generator.generateKey());
        return cipher;
    }

    private static boolean keyUnlocked(BiometricPrompt.CryptoObject crypto) {
        try {
            return crypto != null && crypto.getCipher() != null && crypto.getCipher().doFinal(new byte[16]).length > 0;
        } catch (GeneralSecurityException | RuntimeException e) {
            return false;
        }
    }

    static synchronized void ownerAnswered(int request, boolean confirmed) {
        if (request == ownerRequest && ownerState == OWNER_ASKING) ownerState = confirmed ? OWNER_CONFIRMED : OWNER_REFUSED;
    }

    /**
     * The confirm-credential screen answered. Its request code names the check that opened it, so a
     * screen left over from a destroyed activity cannot answer a newer check.
     */
    static synchronized void ownerScreenAnswered(int requestCode, boolean confirmed) {
        if (ownerScreen == 0 || (ownerScreen & CHECK_BITS) != (requestCode & CHECK_BITS)) return;
        ownerAnswered(ownerScreen, confirmed);
        ownerScreen = 0;
    }

    /** The activity that showed the prompt is gone: its prompt closes and its check is refused. */
    static synchronized void ownerPromptGone() {
        if (ownerPrompt != null) ownerPrompt.cancel();
        ownerPrompt = null;
        ownerScreen = 0;
        ownerAnswered(ownerRequest, false);
    }

    /** 0 while check {@code request} runs, then its answer; a newer check refuses it. */
    static synchronized int ownerState(String request) {
        if (!String.valueOf(ownerRequest).equals(request)) return OWNER_REFUSED;
        return ownerState;
    }

    static void releaseStore(String session) {
        MAIN.post(() -> Stores.release(app, session));
    }

    static void openPrivateSpace() {
        MAIN.post(() -> PrivateSpace.open(app));
    }

    static void closePrivateSpace() {
        MAIN.post(() -> {
            if (privateActivity != null) privateActivity.close();
        });
    }

    static void extensionScriptsChanged(String store) {
        MAIN.post(() -> TarnWebView.extensionScriptsChanged(store));
    }

    static void extensionFrameReady(String request, String[] units) {
        MAIN.post(() -> Extensions.ready(request, units));
    }

    static Object[] fetch(String url, long max, String agent, int stallMs, int totalMs) {
        return Fetch.get(url, max, agent, stallMs, totalMs);
    }

    static Object[] post(String url, String[] headers, byte[] body, int stallMs, int totalMs) {
        return Fetch.post(url, headers, body, stallMs, totalMs);
    }

    static String secretGet(String name) throws GeneralSecurityException {
        return Secrets.get(app, name);
    }

    static void secretSet(String name, String value) throws GeneralSecurityException {
        Secrets.set(app, name, value);
    }

    static void secretDelete(String name) throws GeneralSecurityException {
        Secrets.delete(app, name);
    }

    static void onMain(Runnable task) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            task.run();
        } else {
            MAIN.post(task);
        }
    }
}
