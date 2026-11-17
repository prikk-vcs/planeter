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
        date: "2026-11-17",
        commit: "c5dd85c04a0a94c460025543796f43d1698ae4e3",
        sheet_fingerprint: "57d77f1fb0d3689e",
        note: "The owner (a fluent reader) read the 95 messages that were newer than the sign-off of 2026-11-14 - the messages of the \
               file-by-file view (the pages and the API), the two cut-marker messages, the client's help text of `propose` (changed) and \
               its new refusal for `--proposal` with `--title` - and accepted them all as the sheet had them. This record replaces the \
               one of 2026-11-14 (commit 4c53f2ab6d796400245bd8f69eeedf150324dcb3, sheet fingerprint a917d86974a31464: the owner then \
               read twelve messages and the Japanese accessibility statement, and wrote the final wording of one sentence) and the \
               first of 2026-10-31 (commit 37a6c42aff4f3bfdd83b8dd8e45a6a4281dd24c7, sheet fingerprint 54ebae05ad914ab6). The sheet of \
               the commit above has 754 messages, none newer than this sign-off.",
    }),
};
