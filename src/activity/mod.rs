//! How recently a project was worked on, and what does not count as work.
//!
//! Two pieces, split because they answer different questions:
//!
//! - [`IgnoreFile`] parses one project's `.pmanagerignore` and answers
//!   "is this path ignored?" about a single path, with no traversal involved.
//! - [`last_change`] walks the tree and answers "when was this project last
//!   touched?", consulting the rules above as it goes.
//!
//! Only the second is what the 60/120/160-day thresholds are measured
//! against. Neither has any say in what gets stripped or archived -- that
//! comes from the strip profiles.

pub mod ignore_file;
pub mod last_change;

pub use ignore_file::IgnoreFile;
pub use last_change::last_change;
