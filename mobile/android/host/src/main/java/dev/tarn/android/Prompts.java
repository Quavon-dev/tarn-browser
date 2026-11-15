package dev.tarn.android;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLong;

final class Prompts {
    interface Answer {
        void answer(boolean accept, Map<?, ?> args);
    }

    private static final int MAX_PENDING = 4;
    private static final int MAX_PER_PAGE = 10;
    private static final AtomicLong NEXT = new AtomicLong();

    private final Map<Long, Answer> pending = new LinkedHashMap<>();
    private int asked;

    boolean full() {
        return pending.size() >= MAX_PENDING || asked >= MAX_PER_PAGE;
    }

    void newPage() {
        asked = 0;
    }

    long add(Answer answer) {
        long id = NEXT.incrementAndGet();
        asked++;
        pending.put(id, answer);
        return id;
    }

    boolean answer(Map<?, ?> args) {
        if (!(args.get("id") instanceof Number id)) return false;
        Answer answer = pending.remove(id.longValue());
        if (answer == null) return false;
        answer.answer(Boolean.TRUE.equals(args.get("accept")), args);
        return true;
    }

    void declineAll() {
        List<Answer> answers = new ArrayList<>(pending.values());
        pending.clear();
        for (Answer answer : answers) answer.answer(false, Map.of());
    }

    static String text(Map<?, ?> args, String key) {
        Object value = args.get(key);
        return value instanceof String s ? s : null;
    }
}
