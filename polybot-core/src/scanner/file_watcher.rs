use notify::{Config as NotifyConfig, Event, EventKind, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing;

use crate::config::AppConfig;
use crate::scanner::schema::validate_and_create_event_with_max_age;
use polybot_common::errors::PolybotError;
use polybot_common::types::ScannerEvent;

pub struct FileWatcher {
    watch_dir: PathBuf,
    processed_dir: PathBuf,
    signal_max_age_secs: u64,
    sender: mpsc::Sender<ScannerEvent>,
}

impl FileWatcher {
    pub fn new(config: &AppConfig, sender: mpsc::Sender<ScannerEvent>) -> Self {
        Self {
            watch_dir: PathBuf::from(&config.scanner.watch_dir),
            processed_dir: PathBuf::from(&config.scanner.processed_dir),
            signal_max_age_secs: config.scanner.signal_max_age_secs,
            sender,
        }
    }

    pub async fn watch(&self) -> Result<(), PolybotError> {
        std::fs::create_dir_all(&self.watch_dir)
            .map_err(|e| PolybotError::Scanner(format!("Failed to create watch dir: {}", e)))?;
        std::fs::create_dir_all(&self.processed_dir)
            .map_err(|e| PolybotError::Scanner(format!("Failed to create processed dir: {}", e)))?;

        // Process existing files first
        self.process_existing_files().await?;

        let (tx, rx) = std::sync::mpsc::channel::<Event>();

        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            },
            NotifyConfig::default(),
        )
        .map_err(|e| PolybotError::Scanner(format!("Failed to create watcher: {}", e)))?;

        watcher
            .watch(&self.watch_dir, RecursiveMode::NonRecursive)
            .map_err(|e| PolybotError::Scanner(format!("Failed to start watching: {}", e)))?;

        tracing::info!("File watcher started on: {:?}", self.watch_dir);

        // Watch for new files (blocking receive loop)
        while let Ok(event) = rx.recv() {
            if let EventKind::Create(_) = event.kind {
                for path in &event.paths {
                    if let Some(ext) = path.extension() {
                        if ext == "json" {
                            if let Err(e) = self.process_file(path).await {
                                tracing::error!("Error processing file {:?}: {}", path, e);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_existing_files(&self) -> Result<(), PolybotError> {
        let entries = std::fs::read_dir(&self.watch_dir)
            .map_err(|e| PolybotError::Scanner(format!("Failed to read watch dir: {}", e)))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "json" {
                    self.process_file(&path).await?;
                }
            }
        }
        Ok(())
    }

    async fn process_file(&self, path: &Path) -> Result<(), PolybotError> {
        tracing::info!("Processing signal file: {:?}", path);

        let content = std::fs::read_to_string(path)
            .map_err(|e| PolybotError::Scanner(format!("Failed to read file {:?}: {}", path, e)))?;

        let event = validate_and_create_event_with_max_age(&content, self.signal_max_age_secs)?;
        self.sender
            .send(event)
            .await
            .map_err(|_| PolybotError::ChannelClosed)?;

        // Move to processed directory
        let file_name = path.file_name().unwrap_or_default();
        let dest = self.processed_dir.join(file_name);
        if let Err(e) = std::fs::rename(path, &dest) {
            tracing::warn!("Failed to move {:?} to processed: {}", path, e);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn file_watcher_creates_with_config() {
        let config = AppConfig::default();
        let (tx, _rx) = mpsc::channel(256);
        let _watcher = FileWatcher::new(&config, tx);
    }
}
