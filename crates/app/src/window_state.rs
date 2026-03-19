use iced::{Point, Size, window};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const FILE_NAME: &str = "window.json";
const MIN_WIDTH: f32 = 640.0;
const MIN_HEIGHT: f32 = 400.0;
const DEFAULT_WIDTH: f32 = 1280.0;
const DEFAULT_HEIGHT: f32 = 800.0;

/// Persisted window geometry. Saved as JSON in the app data directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_thread_list_width")]
    pub thread_list_width: f32,
    #[serde(default)]
    pub right_sidebar_open: bool,
}

fn default_sidebar_width() -> f32 { 180.0 }
fn default_thread_list_width() -> f32 { 400.0 }

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            x: None,
            y: None,
            maximized: false,
            sidebar_width: default_sidebar_width(),
            thread_list_width: default_thread_list_width(),
            right_sidebar_open: false,
        }
    }
}

impl WindowState {
    /// Load from disk, falling back to defaults on any error.
    pub fn load(data_dir: &std::path::Path) -> Self {
        let path = Self::path(data_dir);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        let Ok(mut state) = serde_json::from_slice::<Self>(&bytes) else {
            return Self::default();
        };
        state.sanitize();
        state
    }

    /// Save to disk. Errors are silently ignored — window state is
    /// best-effort, not critical.
    pub fn save(&self, data_dir: &std::path::Path) {
        let path = Self::path(data_dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Apply this state to iced's window settings.
    pub fn to_window_settings(&self) -> window::Settings {
        let position = match (self.x, self.y) {
            (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => {
                window::Position::Specific(Point::new(x, y))
            }
            _ => window::Position::default(),
        };

        window::Settings {
            size: Size::new(self.width, self.height),
            position,
            maximized: self.maximized,
            min_size: Some(Size::new(MIN_WIDTH, MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        }
    }

    /// Update size from a window resize event.
    pub fn set_size(&mut self, size: Size) {
        self.width = size.width;
        self.height = size.height;
    }

    /// Update position from a window move event.
    pub fn set_position(&mut self, position: Point) {
        self.x = Some(position.x);
        self.y = Some(position.y);
    }

    /// Clamp values to sane ranges.
    fn sanitize(&mut self) {
        self.width = self.width.max(MIN_WIDTH);
        self.height = self.height.max(MIN_HEIGHT);
        self.sidebar_width = self.sidebar_width.max(180.0);  // SIDEBAR_WIDTH default, not SIDEBAR_MIN_WIDTH
        self.thread_list_width = self.thread_list_width.max(250.0);

        // Reject negative positions (off-screen)
        if let Some(x) = self.x
            && x < 0.0
        {
            self.x = None;
        }
        if let Some(y) = self.y
            && y < 0.0
        {
            self.y = None;
        }
    }

    fn path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }
}
