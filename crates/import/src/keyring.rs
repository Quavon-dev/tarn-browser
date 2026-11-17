use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use oo7::dbus::{Error as DbusError, Service};
use zeroize::Zeroizing;

pub(crate) fn password(label: &str) -> Option<Zeroizing<Vec<u8>>> {
    block_on(lookup(label)).ok().flatten()
}

async fn lookup(label: &str) -> std::result::Result<Option<Zeroizing<Vec<u8>>>, DbusError> {
    let service = Service::new().await?;
    let Some(collection) = service.with_alias(Service::DEFAULT_COLLECTION).await? else { return Ok(None) };
    if collection.is_locked().await? {
        collection.unlock(None).await?;
    }
    for item in collection.items().await? {
        if item.label().await? != label {
            continue;
        }
        if item.is_locked().await? {
            item.unlock(None).await?;
        }
        return Ok(Some(Zeroizing::new(item.secret().await?.as_bytes().to_vec())));
    }
    Ok(None)
}

struct Thread(std::thread::Thread);

impl Wake for Thread {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_waits_for_a_wake_from_another_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<Waker>();
        let mut sent = false;
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        std::thread::spawn(move || {
            let waker = rx.recv().unwrap();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            waker.wake();
        });
        let out = block_on(std::future::poll_fn(|cx| {
            if done.load(std::sync::atomic::Ordering::SeqCst) {
                return Poll::Ready(7);
            }
            if !sent {
                tx.send(cx.waker().clone()).unwrap();
                sent = true;
            }
            Poll::Pending
        }));
        assert_eq!(out, 7);
    }
}
