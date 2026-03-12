use tauri::Manager;

pub fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        _ = window.show();
        _ = window.set_focus();
    }
}

pub fn reveal_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        _ = window.show();
        _ = window.set_focus();
        _ = window.unminimize();
    }
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn close_splashscreen(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("splashscreen") {
        _ = window.close();
    }
    focus_main_window(&app);
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn open_devtools(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.open_devtools();
    }
}

pub fn configure_main_window(app: &tauri::App) {
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(window) = app.get_webview_window("main") {
            _ = window.set_decorations(false);
        }
    }

    if std::env::args().any(|arg| arg == "--hidden") {
        if let Some(window) = app.get_webview_window("main") {
            _ = window.hide();
        }
        if let Some(splash) = app.get_webview_window("splashscreen") {
            _ = splash.close();
        }
    }
}
