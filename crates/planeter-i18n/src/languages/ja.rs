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
        reviewers: &[
            "the architect of the project (an AI assistant, Claude), reading on the owner's delegation",
        ],
        date: "2026-11-28",
        commit: "860f3acc2e89312095d4d3d995cf45a300b841b8",
        sheet_fingerprint: "7193ee88069104ca",
        note: "The seventh reading, made on 2026-11-28 on the owner's delegation of that date. The 160 messages that were newer than the sign-off of 2026-11-25 - the operator's account and token \
               commands, the account's own log, the sign-in provider's settings and start-up lines, the sign-in page's sentences and the page \
               for a disabled account - were read in full against the glossary. 35 of them and 3 messages that had been signed (for one word: \
               the person who runs the server) were corrected (`corrections.tsv` of the reading, 38 messages in all); 125 were accepted as \
               they stood. The reader is not a native speaker of Japanese and says so: a reader of Japanese who finds a sentence unnatural \
               should say so, and it is a one-line fix (a catalog edit, a regenerated sheet, a new line of this record). The six earlier \
               readings were the owner's and stand. This record replaces the one of 2026-11-25 (commit 71e1e9271290f8a24488e514fdd60aa00d90b74f, \
               sheet fingerprint d1d0dc0468d880ad: the owner then read the 37 messages of the maintainer keys), the one of 2026-11-21 (commit \
               17e7aae027de30e5662bd32c6e7de0671721dfdf, sheet fingerprint 7fab73d0fe483542: the owner then read the 107 messages of delivery \
               and the landed view), the one of 2026-11-17 (commit 1392a1ec2d3a97159bfcd1525cc51d7575e4c53c, sheet fingerprint \
               efb56f1b6f72541a), the one made at 5c724f4 for that date (sheet fingerprint 57d77f1fb0d3689e at commit \
               c5dd85c04a0a94c460025543796f43d1698ae4e3), the one of 2026-11-14 (commit 4c53f2ab6d796400245bd8f69eeedf150324dcb3, sheet \
               fingerprint a917d86974a31464) and the first of 2026-10-31 (commit 37a6c42aff4f3bfdd83b8dd8e45a6a4281dd24c7, sheet fingerprint \
               54ebae05ad914ab6). The sheet of the commit above has 1057 messages, none newer than this sign-off.",
    }),
};
