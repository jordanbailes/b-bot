use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Error, ErrorKind, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[derive(Debug)]
pub struct GapStore {
    path: PathBuf,
}

#[derive(Debug, Serialize)]
struct GapRecord<'a> {
    timestamp_secs: u64,
    request: &'a str,
    model_response: &'a str,
}

impl Default for GapStore {
    fn default() -> Self {
        Self {
            path: default_gap_store_path(),
        }
    }
}

impl GapStore {
    pub fn append(&self, request: &str, model_response: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let timestamp_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let record = GapRecord {
            timestamp_secs,
            request,
            model_response,
        };

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line =
            serde_json::to_string(&record).map_err(|err| Error::new(ErrorKind::Other, err))?;
        writeln!(file, "{line}")?;
        Ok(())
    }
}

fn default_gap_store_path() -> PathBuf {
    if let Ok(xdg_state_home) = env::var("XDG_STATE_HOME") {
        return PathBuf::from(xdg_state_home)
            .join("bbot")
            .join("tooling-gaps.jsonl");
    }

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("bbot")
        .join("tooling-gaps.jsonl")
}
