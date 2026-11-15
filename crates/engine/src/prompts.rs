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
    Credentials(RcBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>),
    /// Whether to allow (`None` dismisses) and whether to remember it.
    #[cfg(target_os = "macos")]
    Access(Box<dyn FnOnce(Option<bool>, bool)>),
    #[cfg(any(windows, feature = "chromium"))]
    Answer(Answer),
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
            #[cfg(target_os = "macos")]
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
            None => {}
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

    /// Answers an extension's host request for good: Always or Never.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub fn remember(mut self, allow: bool) {
        match self.reply.take() {
            #[cfg(target_os = "macos")]
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
        Some(Reply::Access(done)) => done(None, false),
        #[cfg(any(windows, feature = "chromium"))]
        Some(Reply::Answer(done)) => done(None),
        None => {}
    }
}
