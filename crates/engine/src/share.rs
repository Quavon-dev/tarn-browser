use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{AnyThread, MainThreadMarker, msg_send};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSSharingServicePicker};
use objc2_foundation::{NSArray, NSData, NSRectEdge, NSString, NSURL};

use crate::Host;

thread_local! {
    /// The picker on screen; AppKit does not keep it alive while it shows.
    static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
}

impl Host {
    /// Opens the system share menu for `url` below `x, y, width, height` in this view's
    /// coordinates (points, origin top-left). Call it outside a GPUI update: the menu may run
    /// AppKit's tracking loop.
    pub fn share(&self, url: &str, x: f64, y: f64, width: f64, height: f64) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else { return };
        let items = NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(url))]);
        // SAFETY: the items are an NSURL, which the picker shares as a link.
        let picker = unsafe { NSSharingServicePicker::initWithItems(mtm.alloc(), &items) };
        PICKER.with(|p| p.replace(Some(picker.clone())));
        let edge = if self.view.isFlipped() { NSRectEdge::MaxY } else { NSRectEdge::MinY };
        picker.showRelativeToRect_ofView_preferredEdge(self.rect(x, y, width, height), &self.view, edge);
    }
}

/// The QR code of `text` (error correction M) as rows of dark modules, quiet zone included;
/// `None` when the text is too long for a QR code or CoreImage is missing.
pub fn qr_code(text: &str) -> Option<Vec<Vec<bool>>> {
    // SAFETY: CIFilter's documented key-value interface with an NSData message and an NSString
    // level; the rep renders the filter's CIImage, and its bitmap is read within its bounds.
    unsafe {
        let filter: Option<Retained<AnyObject>> = msg_send![AnyClass::get(c"CIFilter")?, filterWithName: &*NSString::from_str("CIQRCodeGenerator")];
        let filter = filter?;
        let _: () = msg_send![&filter, setValue: &*NSData::with_bytes(text.as_bytes()), forKey: &*NSString::from_str("inputMessage")];
        let _: () = msg_send![&filter, setValue: &*NSString::from_str("M"), forKey: &*NSString::from_str("inputCorrectionLevel")];
        let image: Option<Retained<AnyObject>> = msg_send![&filter, outputImage];
        let rep: Option<Retained<NSBitmapImageRep>> = msg_send![NSBitmapImageRep::alloc(), initWithCIImage: &*image?];
        let rep = rep?;
        let (width, height, row, step) = (rep.pixelsWide() as usize, rep.pixelsHigh() as usize, rep.bytesPerRow() as usize, rep.bitsPerPixel() as usize / 8);
        let data = rep.bitmapData();
        if data.is_null()
            || rep.bitsPerSample() != 8
            || rep.isPlanar()
            || rep.samplesPerPixel() < 3
            || rep.bitmapFormat().contains(NSBitmapFormat::ThirtyTwoBitLittleEndian)
            || width == 0
            || width != height
        {
            return None;
        }
        let pixels = std::slice::from_raw_parts(data, row * height);
        let red = usize::from(rep.bitmapFormat().contains(NSBitmapFormat::AlphaFirst));
        Some((0..height).map(|y| (0..width).map(|x| pixels[y * row + x * step + red] < 128).collect()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::qr_code;

    /// A finder pattern: a dark 7×7 ring, a light ring inside it and a dark 3×3 centre.
    fn finder(code: &[Vec<bool>], top: usize, left: usize) -> bool {
        (0..7).all(|y| {
            (0..7).all(|x| {
                let ring = y.min(x).min(6 - y).min(6 - x);
                code[top + y][left + x] == (ring != 1)
            })
        })
    }

    #[test]
    fn qr_codes_have_their_three_finder_patterns() {
        let code = qr_code("https://example.com/a?b=c").expect("CoreImage makes a QR code");
        let n = code.len();
        let quiet = code.iter().position(|row| row.contains(&true)).unwrap();
        let size = n - 2 * quiet;
        assert!(size >= 21 && (size - 21).is_multiple_of(4), "{size} modules");
        assert!(finder(&code, quiet, quiet) && finder(&code, quiet, n - quiet - 7) && finder(&code, n - quiet - 7, quiet));
        assert!(!finder(&code, n - quiet - 7, n - quiet - 7));
        assert!(qr_code(&"a".repeat(5000)).is_none());
    }
}
