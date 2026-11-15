//! The Windows share sheet for a link, and QR codes.

use std::cell::RefCell;

use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::IDataTransferManagerInterop;
use windows::core::{HSTRING, Ref, factory};

use crate::Host;

thread_local! {
    /// The handler that answers the share sheet, replaced for each link shared.
    static HANDLER: RefCell<Option<(DataTransferManager, i64)>> = const { RefCell::new(None) };
}

impl Host {
    /// Opens Windows' share sheet for `url`; Windows places it itself, so the rectangle the macOS
    /// menu is anchored to is not used.
    pub fn share(&self, url: &str, _x: f64, _y: f64, _width: f64, _height: f64) {
        let _ = share(HWND(self.hwnd), url);
    }
}

fn share(window: HWND, url: &str) -> windows::core::Result<()> {
    let interop = factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    // SAFETY: `window` is a live top-level window of this thread.
    let manager: DataTransferManager = unsafe { interop.GetForWindow(window)? };
    let (link, title) = (Uri::CreateUri(&HSTRING::from(url))?, HSTRING::from(url));
    let token = manager.DataRequested(&TypedEventHandler::new(move |_: Ref<DataTransferManager>, args: Ref<DataRequestedEventArgs>| {
        let data = args.ok()?.Request()?.Data()?;
        data.Properties()?.SetTitle(&title)?;
        data.SetWebLink(&link)
    }))?;
    if let Some((old, token)) = HANDLER.replace(Some((manager, token))) {
        let _ = old.RemoveDataRequested(token);
    }
    // SAFETY: as above.
    unsafe { interop.ShowShareUIForWindow(window) }
}

/// Modules of light margin around a QR code, as its standard asks.
const QUIET_ZONE: usize = 4;

/// The QR code of `text` (error correction M) as rows of dark modules, quiet zone included;
/// `None` when the text is too long for a QR code.
pub fn qr_code(text: &str) -> Option<Vec<Vec<bool>>> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let dark = |x: usize, y: usize| {
        x.checked_sub(QUIET_ZONE).zip(y.checked_sub(QUIET_ZONE)).is_some_and(|(x, y)| x < width && y < width && code[(x, y)] == qrcode::Color::Dark)
    };
    let size = width + 2 * QUIET_ZONE;
    Some((0..size).map(|y| (0..size).map(|x| dark(x, y)).collect()).collect())
}

#[cfg(test)]
mod tests {
    use super::{QUIET_ZONE, qr_code};

    #[test]
    fn a_code_has_its_finder_pattern_inside_the_quiet_zone() {
        let code = qr_code("https://example.com/").unwrap();
        let q = QUIET_ZONE;
        assert!(code.iter().all(|row| row.len() == code.len()));
        assert!(!code[q - 1][q] && code[q][q] && code[q][q + 6] && !code[q + 1][q + 1] && code[q + 3][q + 3]);
        assert!(qr_code(&"x".repeat(5000)).is_none());
    }
}
