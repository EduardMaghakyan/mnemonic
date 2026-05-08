mod config;
mod llama;
mod markdown;
mod notes;
pub mod permissions;
mod schema;
mod slug;

pub use config::{
    AudioSection, Config, HotkeyMode, HotkeySection, ModelSection, PathsSection, CONFIG_RELATIVE,
    DEFAULT_CONFIG_TOML,
};
pub use llama::{
    health_check, is_silent, structure_audio, StructureRequest, StructuringResult, SYSTEM_PROMPT,
};
pub use markdown::{render_note, write_note, NoteContent, NoteMeta, NoteMetaOverrides, WriteResult};
pub use notes::{find_by_id_prefix, load_note, parse_since, walk_notes, FindByPrefix, LoadedNote};
pub use schema::{Entities, NoteStatus, StructuredNote};
pub use slug::title_to_slug;
