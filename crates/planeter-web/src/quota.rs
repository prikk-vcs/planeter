//! The words of a quota refusal (RFC 005 D-9, Q-2). The rule lives in `planeter_core::change`
//! ([`check_quota`](planeter_core::change::check_quota)); this module only maps a refusal to its catalog keys and
//! renders the two sentences — what was refused and what the person can do — in a language. No route uses it yet
//! (part 3b); the catalog tests keep the keys honest meanwhile.

use planeter_core::change::{Quota, QuotaRefusal};

use crate::catalog::{Key, Lang, T};

/// The message and hint keys of a refusal.
pub fn keys(refusal: &QuotaRefusal) -> (Key, Key) {
    match refusal.quota {
        Quota::OpenChanges => (Key::QuotaOpenProposalsMessage, Key::QuotaOpenProposalsHint),
        Quota::AcceptedBytes => (Key::QuotaAcceptedBytesMessage, Key::QuotaAcceptedBytesHint),
        Quota::RevisionsPerChange => (Key::QuotaRevisionsMessage, Key::QuotaRevisionsHint),
        Quota::RepositoryCapacity => (Key::QuotaCapacityMessage, Key::QuotaCapacityHint),
    }
}

/// `(message, hint)` for a refusal, with its limit and current use in the unit the sentence says.
pub fn texts(refusal: &QuotaRefusal, lang: Lang) -> (String, String) {
    let (message, hint) = keys(refusal);
    let (limit, used) = refusal.figures();
    let t = T::new(lang);
    let say = |key| t.msg(key).with("limit", limit).with("used", used).text();
    (say(message), say(hint))
}
