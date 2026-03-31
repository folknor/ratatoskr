//! Linux system font detection via xdg-desktop-portal.
//!
//! Queries `org.freedesktop.portal.Settings.ReadOne` for the
//! `org.gnome.desktop.interface` namespace. Works on GNOME (via
//! xdg-desktop-portal-gnome), KDE (via xdg-desktop-portal-kde), and
//! any other DE with a portal backend that exposes these keys.

use crate::{SystemFonts, parse_font_description};

const NAMESPACE: &str = "org.gnome.desktop.interface";
const KEY_UI_FONT: &str = "font-name";
const KEY_MONOSPACE_FONT: &str = "monospace-font-name";
const KEY_DOCUMENT_FONT: &str = "document-font-name";

/// Read a single setting from xdg-desktop-portal.
///
/// The portal returns `Variant(Variant(String))` — a doubly-wrapped variant.
/// We unwrap both layers to get the inner string.
async fn read_setting(
    proxy: &zbus::proxy::Proxy<'_>,
    namespace: &str,
    key: &str,
) -> Option<String> {
    let reply: zbus::zvariant::OwnedValue = proxy.call("ReadOne", &(namespace, key)).await.ok()?;

    // The portal wraps the value in Variant(Variant(value)).
    // First unwrap: OwnedValue -> Value (outer variant)
    let inner: zbus::zvariant::Value<'_> = reply
        .downcast_ref::<zbus::zvariant::Value<'_>>()
        .ok()?
        .clone();
    // Second unwrap: Value -> String
    let s: &str = inner.downcast_ref().ok()?;
    Some(s.to_string())
}

pub(crate) async fn detect() -> SystemFonts {
    let mut fonts = SystemFonts::default();

    let connection = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::debug!("failed to connect to session D-Bus: {e}");
            return fonts;
        }
    };

    let proxy = match zbus::proxy::Builder::new(&connection)
        .interface("org.freedesktop.portal.Settings")
        .expect("valid interface name")
        .path("/org/freedesktop/portal/desktop")
        .expect("valid path")
        .destination("org.freedesktop.portal.Desktop")
        .expect("valid destination")
        .build()
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!("failed to create portal settings proxy: {e}");
            return fonts;
        }
    };

    if let Some(desc) = read_setting(&proxy, NAMESPACE, KEY_UI_FONT).await {
        fonts.ui = parse_font_description(&desc);
        if fonts.ui.is_some() {
            tracing::debug!("system UI font: {desc}");
        }
    }

    if let Some(desc) = read_setting(&proxy, NAMESPACE, KEY_MONOSPACE_FONT).await {
        fonts.monospace = parse_font_description(&desc);
        if fonts.monospace.is_some() {
            tracing::debug!("system monospace font: {desc}");
        }
    }

    if let Some(desc) = read_setting(&proxy, NAMESPACE, KEY_DOCUMENT_FONT).await {
        fonts.document = parse_font_description(&desc);
        if fonts.document.is_some() {
            tracing::debug!("system document font: {desc}");
        }
    }

    fonts
}
