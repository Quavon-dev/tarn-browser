use std::rc::Weak;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGPoint;
use objc2_foundation::NSObject;
use objc2_ui_kit::{UIEvent, UIEventType, UIResponder, UIView};
use serde_json::Value;

use crate::component::Inner;

define_class!(
    /// The component's view: a touch that lands on the page reports `pressed` and still reaches the page.
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnPageRoot"]
    #[ivars = Weak<Inner>]
    pub struct PageRoot;

    impl PageRoot {
        #[unsafe(method_id(hitTest:withEvent:))]
        fn hit_test(&self, point: CGPoint, event: Option<&UIEvent>) -> Option<Retained<UIView>> {
            // SAFETY: calling the superclass implementation with the arguments UIKit passed.
            let hit: Option<Retained<UIView>> = unsafe { msg_send![super(self), hitTest: point, withEvent: event] };
            if hit.is_some()
                && event.is_some_and(|e| e.r#type() == UIEventType::Touches)
                && let Some(inner) = self.ivars().upgrade()
            {
                inner.emit("pressed", Value::Null);
            }
            hit
        }
    }
);

impl PageRoot {
    pub fn view(mtm: MainThreadMarker, inner: Weak<Inner>) -> Retained<UIView> {
        let this = Self::alloc(mtm).set_ivars(inner);
        // SAFETY: UIView's `init` on a freshly allocated instance.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.into_super()
    }
}
