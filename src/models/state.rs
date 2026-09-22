use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectState {
    Registered,
    Stripped,
    Archived { archive_path: PathBuf },
}

impl ProjectState {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Stripped => "stripped",
            Self::Archived { .. } => "archived",
        }
    }

    pub fn archive_path(&self) -> Option<&Path> {
        match self {
            Self::Registered | Self::Stripped => None,
            Self::Archived { archive_path } => Some(archive_path),
        }
    }
}
