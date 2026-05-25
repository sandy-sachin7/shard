use crate::manifest::FileManifest;
use crate::metadata::{self, MetadataFormat};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Index {
    pub files: HashMap<String, FileManifest>,
}

impl Index {
    pub fn load(path: &Path, _fmt: &MetadataFormat) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read(path)?;
        metadata::deserialize(&content)
    }

    pub fn save(&self, path: &Path, fmt: &MetadataFormat) -> Result<()> {
        let content = metadata::serialize(self, fmt);
        fs::write(path, content)?;
        Ok(())
    }
}
