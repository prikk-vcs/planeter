//! English: the source language.

use crate::{Dir, Language, Status, plural};

pub static EN: Language = Language {
    tag: "en",
    endonym: "English",
    dir: Dir::Ltr,
    categories: plural::ONE_OTHER,
    plural: plural::en,
    group_separator: ',',
    decimal_separator: '.',
    status: Status::Source,
};
