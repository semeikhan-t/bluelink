//! BlueLink vault: notes on the filesystem, wiki-link index, markdown preview.

mod error;
mod frontmatter;
mod markdown;
mod meta;
mod path;
mod vault;
mod wiki;

pub use error::VaultError;
pub use frontmatter::{
    frontmatter_line_count, frontmatter_raw_yaml, parse_frontmatter, splice_frontmatter_yaml,
    write_frontmatter,
};
pub use markdown::{
    parse_embed_size, parse_preview, split_wikilinks, PreviewBlock, PreviewDoc, Span,
};
pub use meta::DeviceMeta;
pub use path::{
    is_media_extension, max_bytes_for_rel, validate_folder_rel, validate_rel_path,
    validate_vault_rel_path, ATTACHMENTS_DIR, MAX_MEDIA_BYTES, MAX_NOTE_BYTES, MAX_REL_PATH_BYTES,
    MEDIA_EXTENSIONS,
};
pub use vault::{title_from_content, NoteMeta, Vault};
pub use wiki::{
    block_line, create_path_for_link, extract_wikilinks, fragment_line_range, heading_line,
    link_fragment, normalize_link_target, notes_after_renames, preferred_wikilink_text,
    rewrite_heading_links, rewrite_wikilink_body, rewrite_wikilinks_in_content, LocalGraph,
    WikiIndex,
};
