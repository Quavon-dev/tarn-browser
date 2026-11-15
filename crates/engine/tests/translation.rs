//! Translates with the Translation framework, which answers on the main queue, so the test has its
//! own `main`. Needs macOS 26 with the German and English language packs installed, so it runs
//! only with `TARN_TEST_TRANSLATION=1 cargo test -p engine --features translation --test translation`.

#[cfg(target_os = "macos")]
fn main() {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};

    if std::env::var_os("TARN_TEST_TRANSLATION").is_none() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    engine::translation::translate(&["Guten Morgen".into(), "Danke".into()], "de-DE", "en", move |r| drop(tx.send(r)));
    let deadline = Instant::now() + Duration::from_secs(60);
    let texts = loop {
        if let Ok(result) = rx.try_recv() {
            break result.expect("the language packs are installed");
        }
        assert!(Instant::now() < deadline, "the framework did not answer");
        // SAFETY: Core Foundation's exported constant run-loop mode.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, Duration::from_millis(50), true);
    };
    assert_eq!(texts.len(), 2);
    assert!(texts[0].to_lowercase().contains("morning"), "{texts:?}");

    let (tx, rx) = mpsc::channel();
    engine::translation::translate(&["Bonjour".into()], "tlh", "en", move |r| drop(tx.send(r)));
    loop {
        if let Ok(result) = rx.try_recv() {
            assert!(result.is_err(), "{result:?}");
            break;
        }
        assert!(Instant::now() < deadline, "the framework did not answer");
        // SAFETY: as above.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, Duration::from_millis(50), true);
    }
    println!("translation: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
