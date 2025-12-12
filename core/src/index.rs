use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::manifest::FileManifest;
use anyhow::Result;
use std::path::Path;
use std::fs;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Index {
    pub files: HashMap<String, FileManifest>,
}

impl Index {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        let index = serde_json::from_str(&content)?;
        Ok(index)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}
