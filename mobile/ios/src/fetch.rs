use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use block2::DynBlock;
use browser_core::github::{CONNECT_TIMEOUT_SECS, TIMEOUT_SECS, USER_AGENT};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSHTTPURLResponse, NSNumber, NSObject, NSString, NSURL, NSURLRequest, NSURLRequestCachePolicy, NSURLSession,
    NSURLSessionConfiguration, NSURLSessionDataDelegate, NSURLSessionDataTask, NSURLSessionDelegate, NSURLSessionTask, NSURLSessionTaskDelegate,
};

type Answer = Result<(u16, Option<String>, Vec<u8>), String>;

struct State {
    max: u64,
    body: Vec<u8>,
    too_large: bool,
    done: Option<Sender<Answer>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "TarnFetch"]
    #[ivars = Mutex<State>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSURLSessionDelegate for Delegate {}

    unsafe impl NSURLSessionTaskDelegate for Delegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn redirect(&self, _session: &NSURLSession, _task: &NSURLSessionTask, _response: &NSHTTPURLResponse, _request: &NSURLRequest, handler: &DynBlock<dyn Fn(*mut NSURLRequest)>) {
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn completed(&self, _session: &NSURLSession, task: &NSURLSessionTask, error: Option<&NSError>) {
            let mut state = self.ivars().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let answer = if state.too_large {
                Err("The answer is too large.".to_owned())
            } else if let Some(error) = error {
                Err(error.localizedDescription().to_string())
            } else {
                match task.response().and_then(|r| r.downcast::<NSHTTPURLResponse>().ok()) {
                    Some(http) => {
                        let location = http.valueForHTTPHeaderField(&NSString::from_str("Location")).map(|l| l.to_string());
                        Ok((u16::try_from(http.statusCode()).unwrap_or(0), location, std::mem::take(&mut state.body)))
                    }
                    None => Err("The server sent no HTTP answer.".to_owned()),
                }
            };
            if let Some(done) = state.done.take() {
                drop(done.send(answer));
            }
        }
    }

    unsafe impl NSURLSessionDataDelegate for Delegate {
        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn received(&self, _session: &NSURLSession, task: &NSURLSessionDataTask, data: &NSData) {
            let mut state = self.ivars().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.body.extend_from_slice(&data.to_vec());
            if state.body.len() as u64 > state.max {
                state.too_large = true;
                state.body.clear();
                task.cancel();
            }
        }
    }
);

fn proxy() -> Retained<NSDictionary<NSString, AnyObject>> {
    let Some(port) = option_env!("TARN_TEST_PROXY").and_then(|p| p.strip_prefix("127.0.0.1:")).and_then(|p| p.parse::<u16>().ok()) else {
        return NSDictionary::new();
    };
    let keys = [NSString::from_str("HTTPSEnable"), NSString::from_str("HTTPSProxy"), NSString::from_str("HTTPSPort")];
    let values: [Retained<AnyObject>; 3] = [NSNumber::new_bool(true).into(), NSString::from_str("127.0.0.1").into(), NSNumber::new_u16(port).into()];
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    NSDictionary::from_retained_objects(&keys, &values)
}

pub fn fetch(url: &str, max: u64) -> Answer {
    objc2::rc::autoreleasepool(|_| fetch_once(url, max))
}

fn fetch_once(url: &str, max: u64) -> Answer {
    let link = NSURL::URLWithString(&NSString::from_str(url)).ok_or("not a valid address")?;
    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    config.setHTTPShouldSetCookies(false);
    config.setHTTPCookieStorage(None);
    config.setURLCredentialStorage(None);
    config.setURLCache(None);
    config.setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
    config.setTimeoutIntervalForRequest(CONNECT_TIMEOUT_SECS as f64);
    config.setTimeoutIntervalForResource(TIMEOUT_SECS as f64);
    let headers = NSDictionary::from_retained_objects(&[&*NSString::from_str("User-Agent")], &[NSString::from_str(USER_AGENT)]);
    let proxy = proxy();
    // SAFETY: both dictionaries map NSString keys to property-list values, as the configuration
    // expects; the casts only erase their generic parameters.
    unsafe {
        config.setHTTPAdditionalHeaders(Some(&Retained::cast_unchecked::<NSDictionary>(headers)));
        config.setConnectionProxyDictionary(Some(&Retained::cast_unchecked::<NSDictionary>(proxy)));
    }
    let (sender, receiver) = channel();
    let delegate: Retained<Delegate> = {
        let this = Delegate::alloc().set_ivars(Mutex::new(State { max, body: Vec::new(), too_large: false, done: Some(sender) }));
        // SAFETY: NSObject's designated initializer on a freshly allocated object with its ivars set.
        unsafe { msg_send![super(this), init] }
    };
    // SAFETY: a nil queue makes the session create its own serial delegate queue; the delegate
    // only locks its own state and sends on a channel, which is safe from any thread.
    let session = unsafe { NSURLSession::sessionWithConfiguration_delegate_delegateQueue(&config, Some(ProtocolObject::from_ref(&*delegate)), None) };
    let task = session.dataTaskWithRequest(&NSURLRequest::requestWithURL(&link));
    task.resume();
    let answer = receiver.recv_timeout(Duration::from_secs(TIMEOUT_SECS + 5)).unwrap_or_else(|_| Err("GitHub took too long to answer.".to_owned()));
    session.invalidateAndCancel();
    answer
}
