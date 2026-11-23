use std::cell::Cell;
use std::rc::Rc;

#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::Bool;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSString, NSURLCredential, NSURLCredentialPersistence, NSURLSessionAuthChallengeDisposition};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKPermissionDecision;
#[cfg(target_os = "macos")]
use security_framework::identity::SecIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    Alert,
    Confirm,
    Text {
        default: String,
    },
    Camera,
    Microphone,
    CameraAndMicrophone,
    /// HTTP Basic or Digest authentication.
    Credentials {
        realm: String,
    },
    /// The server asks for a TLS client certificate; `names` are the Keychain identities to pick from.
    ClientCertificate {
        names: Vec<String>,
    },
    /// An extension asks to read and change the prompt's origin (a host); `key` is its install
    /// id or bundle path. Allow without Remember lasts an hour.
    ExtensionAccess {
        extension: String,
        key: String,
    },
    /// A download from a host on the malware list; accepting keeps it, anything else cancels it.
    MaliciousDownload {
        file: String,
    },
}

impl PromptKind {
    pub fn is_permission(&self) -> bool {
        self.permission_key().is_some()
    }

    /// Key used to remember a permission decision per origin.
    pub fn permission_key(&self) -> Option<&'static str> {
        match self {
            PromptKind::Camera => Some("camera"),
            PromptKind::Microphone => Some("microphone"),
            PromptKind::CameraAndMicrophone => Some("camera+microphone"),
            PromptKind::ExtensionAccess { .. } => Some("extension"),
            _ => None,
        }
    }
}

/// Chromium's answer: `None` declines, otherwise the text and, for authentication, the credentials.
#[cfg(any(windows, feature = "chromium"))]
pub(crate) type Answer = Box<dyn FnOnce(Option<(String, Option<(String, String)>)>)>;

pub(crate) enum Reply {
    #[cfg(target_os = "macos")]
    Alert(RcBlock<dyn Fn()>),
    #[cfg(target_os = "macos")]
    Confirm(RcBlock<dyn Fn(Bool)>),
    #[cfg(target_os = "macos")]
    Text(RcBlock<dyn Fn(*mut NSString)>),
    #[cfg(target_os = "macos")]
    Permission(RcBlock<dyn Fn(WKPermissionDecision)>),
    #[cfg(target_os = "macos")]
    Credentials(AuthBlock),
    #[cfg(target_os = "macos")]
    Certificate(AuthBlock, CertificateAsk),
    /// Whether to allow (`None` dismisses) and whether to remember it.
    Access(Box<dyn FnOnce(Option<bool>, bool)>),
    #[cfg(any(windows, feature = "chromium"))]
    Answer(Answer),
}

#[cfg(target_os = "macos")]
pub(crate) type AuthBlock = RcBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>;

/// A view's client certificate answers for the page being loaded or shown, per origin: the
/// identity picked or `None` for none, and the challenges that arrived while the origin's prompt
/// was open. `page` counts main-frame navigations, so an old page's prompt answers no new page.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub(crate) struct Certificates {
    pub answers: std::collections::HashMap<String, Option<SecIdentity>>,
    /// Each waiting challenge with the page it came from.
    pub waiting: std::collections::HashMap<String, Vec<(u64, AuthBlock)>>,
    pub page: u64,
}

#[cfg(target_os = "macos")]
impl Certificates {
    /// A new page asks again.
    pub(crate) fn next_page(&mut self) {
        self.answers.clear();
        self.page += 1;
    }

    /// Answers `done` from what this page already settled for `origin`, or queues it behind the
    /// open prompt; `false` when the user still has to be asked.
    pub(crate) fn join(state: &std::cell::RefCell<Self>, origin: &str, done: &AuthBlock) -> bool {
        let known = {
            let mut state = state.borrow_mut();
            let page = state.page;
            match (state.answers.get(origin).cloned(), state.waiting.get_mut(origin)) {
                (Some(answer), _) => Some(answer),
                (None, Some(waiting)) => {
                    waiting.push((page, done.clone()));
                    return true;
                }
                (None, None) => None,
            }
        };
        known.map(|answer| answer_certificate(done, answer.as_ref())).is_some()
    }
}

/// The identities a client certificate prompt offers, and where its answer is kept.
#[cfg(target_os = "macos")]
pub(crate) struct CertificateAsk {
    pub origin: String,
    pub page: u64,
    pub identities: Vec<SecIdentity>,
    pub state: Rc<std::cell::RefCell<Certificates>>,
}

#[cfg(target_os = "macos")]
impl CertificateAsk {
    /// Answers the prompt's challenge and every one waiting on it with `index`'s identity, or
    /// with none, and keeps the answer for the page shown if it asked.
    fn settle(self, done: &AuthBlock, index: Option<usize>) {
        let identity = index.and_then(|i| self.identities.into_iter().nth(i));
        let waiting = {
            let mut state = self.state.borrow_mut();
            let waiting = state.waiting.remove(&self.origin).unwrap_or_default();
            if state.page == self.page || waiting.iter().any(|(page, _)| *page == state.page) {
                state.answers.insert(self.origin.clone(), identity.clone());
            }
            waiting
        };
        for done in std::iter::once(done).chain(waiting.iter().map(|(_, done)| done)) {
            answer_certificate(done, identity.as_ref());
        }
    }
}

/// Sends `identity`, or no certificate, which servers that only ask for one optionally accept.
#[cfg(target_os = "macos")]
pub(crate) fn answer_certificate(done: &AuthBlock, identity: Option<&SecIdentity>) {
    match identity.and_then(crate::delegates::identity_credential) {
        Some(credential) => done.call((NSURLSessionAuthChallengeDisposition::UseCredential, Retained::as_ptr(&credential).cast_mut())),
        None => done.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut())),
    }
}

/// A question a page or server asks, shown tab-modally with its origin. Answer it with
/// `accept`/`decline`; dropping an unanswered prompt declines it, so WebKit is always answered.
pub struct Prompt {
    pub origin: String,
    /// The page's own origin when the request comes from an embedded frame of another origin.
    pub top_origin: Option<String>,
    /// Credentials would be sent without encryption.
    pub insecure: bool,
    pub message: String,
    pub kind: PromptKind,
    pub(crate) reply: Option<Reply>,
    pub(crate) suppress: Rc<Cell<bool>>,
}

impl std::fmt::Debug for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prompt").field("origin", &self.origin).field("kind", &self.kind).finish_non_exhaustive()
    }
}

impl Prompt {
    /// `text` answers a text prompt; `credentials` answers an authentication prompt.
    pub fn accept(mut self, text: &str, credentials: Option<(&str, &str)>) {
        if matches!(self.kind, PromptKind::ClientCertificate { .. }) {
            return self.decline();
        }
        match self.reply.take() {
            #[cfg(target_os = "macos")]
            Some(Reply::Alert(done)) => done.call(()),
            #[cfg(target_os = "macos")]
            Some(Reply::Confirm(done)) => done.call((Bool::YES,)),
            #[cfg(target_os = "macos")]
            Some(Reply::Text(done)) => {
                let text = NSString::from_str(text);
                done.call((Retained::as_ptr(&text).cast_mut(),));
            }
            #[cfg(target_os = "macos")]
            Some(Reply::Permission(done)) => done.call((WKPermissionDecision::Grant,)),
            Some(Reply::Access(done)) => done(Some(true), false),
            #[cfg(target_os = "macos")]
            Some(Reply::Credentials(done)) => match credentials {
                Some((user, password)) => {
                    let credential = NSURLCredential::credentialWithUser_password_persistence(
                        &NSString::from_str(user),
                        &NSString::from_str(password),
                        NSURLCredentialPersistence::ForSession,
                    );
                    done.call((NSURLSessionAuthChallengeDisposition::UseCredential, Retained::as_ptr(&credential).cast_mut()));
                }
                None => done.call((NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge, std::ptr::null_mut())),
            },
            #[cfg(any(windows, feature = "chromium"))]
            Some(Reply::Answer(done)) => done(Some((text.to_owned(), credentials.map(|(u, p)| (u.to_owned(), p.to_owned()))))),
            #[cfg(target_os = "macos")]
            other @ Some(Reply::Certificate(..)) => decline(other),
            None => {}
        }
    }

    /// Answers a client certificate prompt with the identity at `index` of its `names`; any
    /// other index sends none.
    pub fn choose(mut self, index: usize) {
        match self.reply.take() {
            #[cfg(target_os = "macos")]
            Some(Reply::Certificate(done, ask)) => ask.settle(&done, Some(index)),
            #[cfg(any(windows, feature = "chromium"))]
            Some(Reply::Answer(done)) => done(Some((index.to_string(), None))),
            other => {
                let _ = index;
                decline(other)
            }
        }
    }

    #[cfg(any(windows, feature = "chromium"))]
    pub(crate) fn answered(
        origin: String,
        top_origin: Option<String>,
        insecure: bool,
        message: String,
        kind: PromptKind,
        reply: Answer,
        suppress: Rc<Cell<bool>>,
    ) -> Self {
        Prompt { origin, top_origin, insecure, message, kind, reply: Some(Reply::Answer(reply)), suppress }
    }

    /// A Tarn extension's request to read and change `host`, raised by the shell; `reply` gets
    /// whether to allow (`None` when dismissed) and whether to remember it.
    pub fn extension_access(extension: String, key: String, host: String, reply: impl FnOnce(Option<bool>, bool) + 'static) -> Self {
        let kind = PromptKind::ExtensionAccess { extension, key };
        Prompt {
            origin: host,
            top_origin: None,
            insecure: false,
            message: String::new(),
            kind,
            reply: Some(Reply::Access(Box::new(reply))),
            suppress: Rc::default(),
        }
    }

    /// Answers an extension's host request for good: Always or Never.
    pub fn remember(mut self, allow: bool) {
        match self.reply.take() {
            Some(Reply::Access(done)) => done(Some(allow), true),
            other => decline(other),
        }
    }

    pub fn decline(mut self) {
        decline(self.reply.take());
    }

    /// Answers this prompt negatively and every further dialog from the page until it navigates.
    pub fn decline_and_silence_page(self) {
        self.suppress.set(true);
        self.decline();
    }
}

impl Drop for Prompt {
    fn drop(&mut self) {
        decline(self.reply.take());
    }
}

#[cfg(all(test, any(windows, feature = "chromium")))]
mod tests {
    use super::*;

    #[test]
    fn an_answered_prompt_declines_when_dropped_and_passes_text_when_accepted() {
        let got = Rc::new(std::cell::RefCell::new(Vec::new()));
        let prompt = |got: Rc<std::cell::RefCell<Vec<Option<String>>>>| {
            let reply: Answer = Box::new(move |a| got.borrow_mut().push(a.map(|(t, _)| t)));
            Prompt::answered("https://a.test".into(), None, false, String::new(), PromptKind::Text { default: String::new() }, reply, Rc::default())
        };
        drop(prompt(got.clone()));
        prompt(got.clone()).accept("hi", None);
        prompt(got.clone()).decline();
        assert_eq!(*got.borrow(), [None, Some("hi".to_owned()), None]);
    }

    #[test]
    fn a_certificate_prompt_passes_only_the_picked_index() {
        let got = Rc::new(std::cell::RefCell::new(Vec::new()));
        let prompt = |got: Rc<std::cell::RefCell<Vec<Option<String>>>>| {
            let reply: Answer = Box::new(move |a| got.borrow_mut().push(a.map(|(t, _)| t)));
            let kind = PromptKind::ClientCertificate { names: vec!["A".into(), "B".into()] };
            Prompt::answered("https://a.test".into(), None, false, String::new(), kind, reply, Rc::default())
        };
        prompt(got.clone()).choose(1);
        prompt(got.clone()).accept("1", None);
        prompt(got.clone()).decline();
        assert_eq!(*got.borrow(), [Some("1".to_owned()), None, None]);
    }
}

pub(crate) fn decline(reply: Option<Reply>) {
    match reply {
        #[cfg(target_os = "macos")]
        Some(Reply::Alert(done)) => done.call(()),
        #[cfg(target_os = "macos")]
        Some(Reply::Confirm(done)) => done.call((Bool::NO,)),
        #[cfg(target_os = "macos")]
        Some(Reply::Text(done)) => done.call((std::ptr::null_mut(),)),
        #[cfg(target_os = "macos")]
        Some(Reply::Permission(done)) => done.call((WKPermissionDecision::Deny,)),
        #[cfg(target_os = "macos")]
        Some(Reply::Credentials(done)) => done.call((NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge, std::ptr::null_mut())),
        #[cfg(target_os = "macos")]
        Some(Reply::Certificate(done, ask)) => ask.settle(&done, None),
        Some(Reply::Access(done)) => done(None, false),
        #[cfg(any(windows, feature = "chromium"))]
        Some(Reply::Answer(done)) => done(None),
        None => {}
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;

    #[test]
    fn an_extension_access_prompt_answers_once() {
        type Answers = Rc<std::cell::RefCell<Vec<(Option<bool>, bool)>>>;
        let got: Answers = Rc::default();
        let prompt = |got: &Answers| {
            let got = got.clone();
            Prompt::extension_access("Ext".into(), "id".into(), "a.test".into(), move |allow, remember| got.borrow_mut().push((allow, remember)))
        };
        drop(prompt(&got));
        prompt(&got).accept("", None);
        prompt(&got).remember(false);
        prompt(&got).decline();
        assert_eq!(*got.borrow(), [(None, false), (Some(true), false), (Some(false), true), (None, false)]);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod certificate_tests {
    use super::*;

    type Got = Rc<std::cell::RefCell<Vec<NSURLSessionAuthChallengeDisposition>>>;

    fn recorder(got: &Got) -> AuthBlock {
        let got = got.clone();
        RcBlock::new(move |d: NSURLSessionAuthChallengeDisposition, _: *mut NSURLCredential| got.borrow_mut().push(d))
    }

    fn prompt(done: AuthBlock, state: &Rc<std::cell::RefCell<Certificates>>) -> Prompt {
        state.borrow_mut().waiting.insert("https://a.test".into(), Vec::new());
        let page = state.borrow().page;
        let ask = CertificateAsk { origin: "https://a.test".into(), page, identities: Vec::new(), state: state.clone() };
        Prompt {
            origin: "https://a.test".into(),
            top_origin: None,
            insecure: false,
            message: String::new(),
            kind: PromptKind::ClientCertificate { names: Vec::new() },
            reply: Some(Reply::Certificate(done, ask)),
            suppress: Rc::default(),
        }
    }

    #[test]
    fn a_certificate_prompt_answers_once_and_sends_none_unless_one_is_picked() {
        let got: Got = Rc::default();
        let answered: [fn(Prompt); 5] = [drop, Prompt::decline, |p| p.accept("", None), |p| p.choose(0), Prompt::decline_and_silence_page];
        for answer in answered {
            answer(prompt(recorder(&got), &Rc::default()));
        }
        assert_eq!(*got.borrow(), [NSURLSessionAuthChallengeDisposition::PerformDefaultHandling; 5]);
    }

    #[test]
    fn challenges_while_the_prompt_is_open_share_its_answer_and_later_ones_reuse_it() {
        let (got, state): (Got, Rc<std::cell::RefCell<Certificates>>) = Default::default();
        let open = prompt(recorder(&got), &state);
        assert!(Certificates::join(&state, "https://a.test", &recorder(&got)));
        assert!(!Certificates::join(&state, "https://b.test", &recorder(&got)));
        assert!(got.borrow().is_empty());
        open.decline();
        assert_eq!(got.borrow().len(), 2);
        assert!(Certificates::join(&state, "https://a.test", &recorder(&got)));
        assert_eq!(*got.borrow(), [NSURLSessionAuthChallengeDisposition::PerformDefaultHandling; 3]);
    }

    #[test]
    fn an_old_pages_prompt_leaves_no_answer_for_the_next_page() {
        let (got, state): (Got, Rc<std::cell::RefCell<Certificates>>) = Default::default();
        let open = prompt(recorder(&got), &state);
        state.borrow_mut().next_page();
        open.decline();
        assert_eq!(got.borrow().len(), 1);
        assert!(!Certificates::join(&state, "https://a.test", &recorder(&got)));
    }

    #[test]
    fn a_new_page_that_joined_the_open_prompt_keeps_its_answer() {
        let (got, state): (Got, Rc<std::cell::RefCell<Certificates>>) = Default::default();
        let open = prompt(recorder(&got), &state);
        state.borrow_mut().next_page();
        assert!(Certificates::join(&state, "https://a.test", &recorder(&got)));
        open.decline();
        assert_eq!(got.borrow().len(), 2);
        assert!(Certificates::join(&state, "https://a.test", &recorder(&got)));
        assert_eq!(got.borrow().len(), 3);
    }
}
