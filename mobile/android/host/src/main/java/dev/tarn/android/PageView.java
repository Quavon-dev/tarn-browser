package dev.tarn.android;

import android.content.Context;
import android.graphics.Rect;
import android.view.ActionMode;
import android.view.Menu;
import android.view.MenuItem;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.webkit.WebView;
import java.util.List;
import java.util.function.Consumer;
import java.util.function.Supplier;

final class PageView extends WebView {
    private static final int ASK = 0x7a5e0000;
    private static final int ORDER = 200;
    private final boolean noPersonalizedLearning;
    private Supplier<List<String>> assistantMenu = () -> null;
    private Consumer<String> askAssistant = command -> {};

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

    void setAssistantMenu(Supplier<List<String>> commands, Consumer<String> ask) {
        assistantMenu = commands;
        askAssistant = ask;
    }

    @Override
    public ActionMode startActionMode(ActionMode.Callback callback, int type) {
        return super.startActionMode(new AssistantCallback(callback), type);
    }

    /** The floating toolbar shows no submenus, so the commands follow in its overflow. */
    private final class AssistantCallback extends ActionMode.Callback2 {
        private final ActionMode.Callback wrapped;
        private List<String> commands;

        AssistantCallback(ActionMode.Callback wrapped) {
            this.wrapped = wrapped;
        }

        @Override
        public boolean onCreateActionMode(ActionMode mode, Menu menu) {
            if (!wrapped.onCreateActionMode(mode, menu)) return false;
            commands = assistantMenu.get();
            if (commands == null) return true;
            menu.add(Menu.NONE, ASK, ORDER, Native.askSelectionLabel());
            String to = Native.assistantCommandsLabel();
            for (int i = 0; i < commands.size(); i++) menu.add(Menu.NONE, ASK + 1 + i, ORDER + 1 + i, to + " " + commands.get(i));
            return true;
        }

        @Override
        public boolean onPrepareActionMode(ActionMode mode, Menu menu) {
            return wrapped.onPrepareActionMode(mode, menu);
        }

        @Override
        public boolean onActionItemClicked(ActionMode mode, MenuItem item) {
            int at = item.getItemId() - ASK;
            if (commands == null || at < 0 || at > commands.size()) return wrapped.onActionItemClicked(mode, item);
            askAssistant.accept(at == 0 ? null : commands.get(at - 1));
            mode.finish();
            return true;
        }

        @Override
        public void onDestroyActionMode(ActionMode mode) {
            wrapped.onDestroyActionMode(mode);
        }

        @Override
        public void onGetContentRect(ActionMode mode, View view, Rect outRect) {
            if (wrapped instanceof ActionMode.Callback2 positioned) {
                positioned.onGetContentRect(mode, view, outRect);
            } else {
                super.onGetContentRect(mode, view, outRect);
            }
        }
    }
}
