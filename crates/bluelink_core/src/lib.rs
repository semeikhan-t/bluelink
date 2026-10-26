//! BlueLink vault: notes on the filesystem, wiki-link index, markdown preview.

mod error;
mod markdown;
mod meta;
mod path;
mod vault;
mod wiki;

pub use error::VaultError;
pub use markdown::{parse_preview, split_wikilinks, PreviewBlock, PreviewDoc, Span};
pub use meta::DeviceMeta;
pub use path::{validate_folder_rel, validate_rel_path, MAX_NOTE_BYTES, MAX_REL_PATH_BYTES};
pub use vault::{title_from_content, NoteMeta, Vault};
pub use wiki::{
    create_path_for_link, extract_wikilinks, normalize_link_target, notes_after_renames,
    preferred_wikilink_text, rewrite_wikilink_body, rewrite_wikilinks_in_content, LocalGraph,
    WikiIndex,
};
