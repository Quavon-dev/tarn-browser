#![forbid(unsafe_code)]

#[cfg(all(target_os = "macos", feature = "chromium"))]
fn main() {
    let args = cef::args::Args::new();
    let sandbox = std::env::args().all(|a| a != "--no-sandbox").then(|| {
        let mut sandbox = cef::sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        sandbox
    });
    let exe = std::env::current_exe().expect("helper path");
    let loader = cef::library_loader::LibraryLoader::new(&exe, true);
    if !loader.load() {
        std::process::exit(1);
    }
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let code = cef::execute_process(Some(args.as_main_args()), Some(&mut renderer::ChildApp::new()), std::ptr::null_mut());
    drop(loader);
    drop(sandbox);
    std::process::exit(code);
}

/// A renderer tells the browser process its id when it starts a main-frame document, so the tab's
/// memory can be measured; the browser checks the id names one of Tarn's helpers.
#[cfg(all(target_os = "macos", feature = "chromium"))]
mod renderer {
    use cef::*;

    /// Matches `RENDERER_PID` in crates/engine/src/chromium/runtime.rs.
    const RENDERER_PID: &str = "tarn.renderer-pid";

    wrap_app! {
        pub struct ChildApp;

        impl App {
            fn render_process_handler(&self) -> Option<RenderProcessHandler> {
                Some(ChildRenderer::new())
            }
        }
    }

    wrap_render_process_handler! {
        struct ChildRenderer;

        impl RenderProcessHandler {
            fn load_handler(&self) -> Option<LoadHandler> {
                Some(ChildLoad::new())
            }
        }
    }

    wrap_load_handler! {
        struct ChildLoad;

        impl LoadHandler {
            fn on_load_start(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _transition_type: TransitionType) {
                let Some(frame) = frame.filter(|f| f.is_main() == 1) else { return };
                let Some(mut message) = process_message_create(Some(&CefString::from(RENDERER_PID))) else { return };
                let Ok(pid) = i32::try_from(std::process::id()) else { return };
                if message.argument_list().is_some_and(|a| a.set_int(0, pid) == 1) {
                    frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
                }
            }
        }
    }
}

#[cfg(not(all(target_os = "macos", feature = "chromium")))]
fn main() {
    std::process::exit(1);
}
