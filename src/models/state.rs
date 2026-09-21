#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectState {
    Registered,
    Stripped,
    Archived { archive_path: String },
}

impl ProjectState {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Stripped => "stripped",
            Self::Archived { .. } => "archived",
        }
    }

    pub fn archive_path(&self) -> Option<&str> {
        match self {
            Self::Registered | Self::Stripped => None,
            Self::Archived { archive_path } => Some(archive_path),
        }
    }
}
