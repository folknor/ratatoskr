use tauri::{Emitter, Manager};

#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconId},
};

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn set_tray_tooltip(app: tauri::AppHandle, tooltip: String) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let tray = app
            .tray_by_id(&TrayIconId::new("main-tray"))
            .ok_or_else(|| "Tray icon not found".to_string())?;
        tray.set_tooltip(Some(&tooltip)).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "linux")]
    {
        _ = tooltip;
        _ = app;
        log::debug!("set_tray_tooltip is not supported on Linux (KSNI tray)");
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Ratatoskr", true, None::<&str>)?;
    let check_mail = MenuItem::with_id(app, "check_mail", "Check for Mail", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &check_mail, &quit])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("app should have a default icon configured in tauri.conf.json bundle");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Ratatoskr")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => crate::window::focus_main_window(app),
            "check_mail" => {
                if let Some(window) = app.get_webview_window("main") {
                    _ = window.emit("tray-check-mail", ());
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                crate::window::focus_main_window(&tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg(target_os = "linux")]
pub fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tray_item::{IconSource, TrayItem};

    let app_handle = app.handle().clone();

    std::thread::spawn(move || {
        let mut tray = match TrayItem::new("Ratatoskr", IconSource::Resource("mail-read")) {
            Ok(tray) => tray,
            Err(error) => {
                log::warn!("Failed to create system tray: {error}");
                return;
            }
        };

        let app_handle_show = app_handle.clone();
        if let Err(error) = tray.add_menu_item("Show Ratatoskr", move || {
            crate::window::focus_main_window(&app_handle_show);
        }) {
            log::warn!("Failed to add tray menu item 'Show Ratatoskr': {error}");
        }

        let app_handle_check = app_handle.clone();
        if let Err(error) = tray.add_menu_item("Check for Mail", move || {
            if let Some(window) = app_handle_check.get_webview_window("main") {
                _ = window.emit("tray-check-mail", ());
            }
        }) {
            log::warn!("Failed to add tray menu item 'Check for Mail': {error}");
        }

        let app_handle_quit = app_handle.clone();
        if let Err(error) = tray.add_menu_item("Quit", move || {
            app_handle_quit.exit(0);
        }) {
            log::warn!("Failed to add tray menu item 'Quit': {error}");
        }

        loop {
            std::thread::park();
        }
    });

    Ok(())
}
