//! macOS ends a bare binary that asks for Contacts (tish-macos's `macos.contacts`) without a usage
//! description, and only Moo.app has an Info.plist file; this section gives the unbundled binary
//! one. No bundle id: that would change how other permissions and notifications see the dev build.

#[used]
#[link_section = "__TEXT,__info_plist"]
static INFO_PLIST: [u8; include_bytes!("info.plist").len()] = *include_bytes!("info.plist");
