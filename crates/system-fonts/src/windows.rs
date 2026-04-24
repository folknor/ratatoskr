//! Windows system font detection via `SystemParametersInfo`.
//!
//! Reads `NONCLIENTMETRICS` to get the system UI font (typically Segoe UI)
//! and its size.

use crate::{SystemFont, SystemFonts};

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
pub(crate) async fn detect() -> SystemFonts {
    let mut fonts = SystemFonts::default();

    // SAFETY: NONCLIENTMETRICSW is a plain data struct, zero-init is valid.
    let mut metrics: windows_sys::Win32::UI::WindowsAndMessaging::NONCLIENTMETRICSW =
        unsafe { std::mem::zeroed() };
    metrics.cbSize = std::mem::size_of::<
        windows_sys::Win32::UI::WindowsAndMessaging::NONCLIENTMETRICSW,
    >() as u32;

    // SAFETY: SystemParametersInfoW with SPI_GETNONCLIENTMETRICS reads system
    // metrics into the provided buffer. The buffer is correctly sized.
    let success = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows_sys::Win32::UI::WindowsAndMessaging::SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut _ as *mut std::ffi::c_void,
            0,
        )
    };

    if success == 0 {
        tracing::debug!("SystemParametersInfoW failed");
        return fonts;
    }

    // lfMessageFont is the font used for message boxes and dialogs - the
    // standard UI font on Windows.
    let logfont = &metrics.lfMessageFont;
    if let Some(family) = logfont_family_name(logfont) {
        // lfHeight is in logical units; negative means character height in pixels.
        // Convert to approximate point size (assuming 96 DPI).
        let height = logfont.lfHeight.unsigned_abs();
        let pt_size = (height as f32) * 72.0 / 96.0;

        tracing::debug!("system UI font: {family} {pt_size}pt");
        fonts.ui = Some(SystemFont {
            family,
            size: Some(pt_size),
        });
    }

    // lfStatusFont is sometimes different (used in status bars), but for
    // monospace we'd need to look elsewhere. Windows doesn't have a single
    // "monospace" system setting like GNOME does, so we leave it as None
    // and let the caller use their bundled monospace font.

    fonts
}

/// Extract the font family name from a LOGFONTW's null-terminated UTF-16 `lfFaceName`.
fn logfont_family_name(logfont: &windows_sys::Win32::Graphics::Gdi::LOGFONTW) -> Option<String> {
    let face = &logfont.lfFaceName;
    let len = face.iter().position(|&c| c == 0).unwrap_or(face.len());
    let name = String::from_utf16_lossy(&face[..len]);
    if name.is_empty() { None } else { Some(name) }
}
