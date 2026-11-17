package dev.tarn.android;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

public class DownloadsTest {
    @Rule public final TemporaryFolder temp = new TemporaryFolder();

    @Test
    public void downloadsOfOneNameFinishingTogetherKeepEveryFile() throws Exception {
        int count = 16;
        File dir = temp.getRoot();
        ExecutorService pool = Executors.newFixedThreadPool(count);
        try {
            for (int round = 0; round < 20; round++) {
                File target = new File(dir, "round" + round);
                assertTrue(target.mkdir());
                CountDownLatch start = new CountDownLatch(1);
                List<Future<File>> placed = new ArrayList<>();
                for (int i = 0; i < count; i++) {
                    File part = File.createTempFile("download", ".part", dir);
                    Files.write(part.toPath(), ("file " + i).getBytes(StandardCharsets.UTF_8));
                    placed.add(pool.submit(() -> {
                        start.await();
                        return Downloads.place(part, target, "a.zip");
                    }));
                }
                start.countDown();
                Set<String> contents = new HashSet<>();
                for (Future<File> file : placed) contents.add(new String(Files.readAllBytes(file.get().toPath()), StandardCharsets.UTF_8));
                assertEquals(count, contents.size());
                assertEquals(count, target.list().length);
            }
        } finally {
            pool.shutdownNow();
        }
    }
}
