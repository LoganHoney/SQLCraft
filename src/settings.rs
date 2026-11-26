//! Saved connections and preferences, plus passwords in the system keyring.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::db::ConnectionInfo;
use crate::theme::ThemeKind;

const KEYRING_SERVICE: &str = "sqlcraft";
const MAX_RECENT: usize = 15;

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub recent: Vec<ConnectionInfo>,
    pub theme: ThemeKind,
    /// Rows kept per result set in the grid.
    pub max_rows: usize,
    pub font_size: f32,
    pub results_to_text: bool,
    pub recent_files: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            theme: ThemeKind::Dark,
            max_rows: 1_000_000,
            font_size: 14.0,
            results_to_text: false,
            recent_files: Vec::new(),
        }
    }
}

fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "sqlcraft").map(|d| d.config_dir().join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }

    /// Moves `info` to the top of the recent list.
    pub fn remember(&mut self, info: &ConnectionInfo) {
        let key = info.key();
        self.recent.retain(|c| c.key() != key);
        self.recent.insert(0, info.clone());
        self.recent.truncate(MAX_RECENT);
        self.save();
    }

    pub fn remember_file(&mut self, path: &std::path::Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(10);
        self.save();
    }

    pub fn forget(&mut self, key: &str) {
        self.recent.retain(|c| c.key() != key);
        self.save();
    }
}

// The keyring talks to the Secret Service over D-Bus and can block, so the
// app calls these from a blocking task, never from the UI thread.

pub fn load_password(key: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, key).ok()?.get_password().ok()
}

pub fn store_password(key: &str, password: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, key)
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

pub fn delete_password(key: &str) {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, key) {
        let _ = e.delete_credential();
    }
}
