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
        date: "2026-11-14",
        commit: "4c53f2ab6d796400245bd8f69eeedf150324dcb3",
        sheet_fingerprint: "a917d86974a31464",
        note: "The owner (a fluent reader) read the twelve messages that were newer than the first sign-off and the Japanese accessibility \
               statement, wrote the final wording of one of them (the legend of the badge \"not listed here\") and accepted all the \
               others. This record replaces the first one, of 2026-10-31 (commit 37a6c42aff4f3bfdd83b8dd8e45a6a4281dd24c7, sheet \
               fingerprint 54ebae05ad914ab6: the owner approved the generated sheet 6ba30960fe807d98, 653 messages, then the final \
               corrections), whose baselines left out eight messages the owner had not yet read. The sheet of the commit above has 661 \
               messages, none newer than this sign-off.",
    }),
};
