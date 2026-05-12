mod config;
pub mod inbox;
mod intent;
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
pub use intent::{extract_intent, IntentRequest, IntentResult};
pub use llama::{
    health_check, is_silent, structure_audio, StructureRequest, StructuringResult, SYSTEM_PROMPT,
};
pub use markdown::{
    append_entry, count_entries, AppendResult, EntryOverrides, ExecutedIntent, NoteContent,
};
pub use notes::{load_day, parse_since, walk_days, DailyFile};
pub use schema::{ImageNote, Intent, NoteStatus, StructuredNote};
pub use slug::title_to_slug;
