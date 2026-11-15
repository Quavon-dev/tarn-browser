mod browser;
mod controls;
mod data;
mod fetch;
mod input;
mod memory;
mod shortcuts;
mod snapshot;
mod speech;
mod ui;
mod webview;

use std::rc::Rc;

use tarn_nsh_shared::controls::{Density, set_density};
use tarn_nsh_shared::{icons, theme};

use nsh_platform_linux as platform;
use nsh_platform_linux::{ComponentContext, Components, NativeComponent};

const APP_ID: &str = "dev.tarn.Tarn";

fn main() -> impl std::process::Termination {
    set_density(Density::Desktop);
    let root = data::root();
    if let Err(e) = webview::session::create_private_dir(&root) {
        eprintln!("tarn: cannot create {}: {e}", root.display());
    }
    let components = Components::default();
    components.register(webview::NAME, Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> { Box::new(webview::TarnWebView::new(context)) }));
    let code = platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<ui::App>()),
        platform::AppOptions {
            title: "Tarn".to_owned(),
            app_id: APP_ID.to_owned(),
            size: (1280, 800),
            components,
            data_dir: Some(root.join("nsh")),
            ..Default::default()
        },
    );
    speech::stop();
    code
}
