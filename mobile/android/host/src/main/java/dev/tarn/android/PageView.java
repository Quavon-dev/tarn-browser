package dev.tarn.android;

import android.content.Context;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.webkit.WebView;

final class PageView extends WebView {
    private final boolean noPersonalizedLearning;

    PageView(Context context, boolean noPersonalizedLearning) {
        super(context);
        this.noPersonalizedLearning = noPersonalizedLearning;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo info) {
        InputConnection connection = super.onCreateInputConnection(info);
        if (noPersonalizedLearning) info.imeOptions |= EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING;
        return connection;
    }
}
