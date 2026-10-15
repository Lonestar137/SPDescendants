// Vendored from IW4L (https://github.com/vladtrc/iw4L) at commit f026f5f, Apache-2.0.
// See `vendor/iw4/README.md` for provenance and `NOTICE.md` at the repository root.
// Unmodified upstream source apart from this notice and the crate's own manifest.

#![no_std]
#![forbid(unsafe_code)]

pub mod decl_type;
pub mod usage;

pub use decl_type::DeclType;
pub use usage::{
    D3DDECLUSAGE_COLOR, D3DDECLUSAGE_DEPTH, D3DDECLUSAGE_NORMAL, D3DDECLUSAGE_POSITION,
    D3DDECLUSAGE_TEXCOORD, Semantic,
};
