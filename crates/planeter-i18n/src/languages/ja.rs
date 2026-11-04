//! Japanese: signed off (RFC 012 D-3f). The record is data on the language; `locales/ja.signed` of each program lists the messages as
//! they were signed.

use crate::{Dir, Language, SignOff, Status, plural};

pub static JA: Language = Language {
    tag: "ja",
    endonym: "日本語",
    dir: Dir::Ltr,
    categories: plural::OTHER_ONLY,
    plural: plural::ja,
    group_separator: ',',
    decimal_separator: '.',
    status: Status::SignedOff(SignOff {
        reviewers: &["the owner"],
        date: "2026-10-31",
        commit: "37a6c42aff4f3bfdd83b8dd8e45a6a4281dd24c7",
        sheet_fingerprint: "54ebae05ad914ab6",
        note: "The owner (a fluent reader; the colleague named in the plan did not take part) approved the generated sheet with fingerprint \
               6ba30960fe807d98 (653 messages), then the final corrections of the commit above. The sheet regenerated at that commit has \
               661 messages: the eight messages of the answers the framework used to write by itself (added by the dev team after the \
               review) and the Japanese version of the accessibility statement had not been read by the owner.",
    }),
};
