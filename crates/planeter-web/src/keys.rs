//! **The keys a repository trusts, and how each came** (RFC 013 D-4): on the repository page, the history page and in the JSON of the refs
//! route. The list is prikk's own (`trust maintainer list`) joined to planeter's record of how each key got there; a key the record lacks
//! says so ("Source not recorded.") and claims nothing. **Nothing here says a key is retired, revoked, expired, rotated or removed**
//! (RFC 013 D-1: planeter never removes a key; a test holds the words).
//!
//! The reader is the export service's (it holds the repositories, the leases and the record); it runs one prikk call under a read lease,
//! admitted by the prikk gate like any other. Where the service has no record of the keys (a server without the transport), nothing is
//! shown: the page then claims nothing.

use serde::Serialize;

use planeter_core::Principal;
use planeter_store::RepositoryRecord;
use planeter_transport::{KeySourceView, RepoKey};

use crate::AppState;
use crate::catalog::{Key, T};

/// What reading the trusted keys of one repository came to.
pub(crate) enum KeysRead {
    /// The server has no reader (the transport is off): nothing is shown and nothing is claimed.
    Off,
    /// prikk or the store did not answer: the page says so, and the JSON carries `null`. **A list from earlier is never shown as current.**
    Unavailable,
    Keys(Vec<RepoKey>),
}

/// Read the keys `record` trusts now. The wait for a prikk slot is bounded like every other page read.
pub(crate) async fn read_record(state: &AppState, record: RepositoryRecord) -> KeysRead {
    let Some(export) = state.transport.as_ref().map(|t| t.export.clone()) else {
        return KeysRead::Off;
    };
    let job = move || export.repo_keys(&record);
    match state
        .gate
        .run_weighted(planeter_core::admission::COST_FLOOR, "trust list", job)
        .await
    {
        Ok(None) => KeysRead::Off,
        Ok(Some(Ok(keys))) => KeysRead::Keys(keys),
        Ok(Some(Err(_))) | Err(_) => KeysRead::Unavailable,
    }
}

/// The keys of `owner/name` for a page: the repository is looked up as the reader (a repository a stranger cannot see is `Off` here, and the
/// page itself is already a 404 then).
pub(crate) async fn read(
    state: &AppState,
    principal: &Principal,
    owner: &str,
    name: &str,
) -> KeysRead {
    if state.transport.is_none() {
        return KeysRead::Off;
    }
    match crate::transport::readable(state, principal, owner, name).await {
        Ok(record) => read_record(state, record).await,
        Err(_) => KeysRead::Off,
    }
}

/// The most keys a repository can trust (RFC 013 D-6), for the documents' checks.
pub fn keys_max() -> usize {
    planeter_transport::KEYS_MAX
}

// ------------------------------------------------------------------------------------------------ the page

/// One key of the list on a page.
pub struct KeyItem {
    pub key_id: String,
    pub fingerprint: String,
    /// The one sentence of how it came.
    pub source: String,
}

/// The section of a page: the list under a native `<details>`.
pub struct KeysBlock {
    pub heading: String,
    /// The number of keys, as the summary shows it beside the heading.
    pub count: Option<String>,
    /// A sentence instead of (or beside) the list: no keys, or the keys cannot be read now.
    pub note: Option<String>,
    pub items: Vec<KeyItem>,
}

/// The one sentence of source of `key`, in `t`'s language. The reason of an operator is text: it goes through the template's escaping.
pub(crate) fn source_sentence(t: &T, key: &RepoKey) -> String {
    let date = || key.recorded_at.map(crate::timefmt::date);
    match &key.source {
        KeySourceView::Creation { backfilled: false } => t.msg(Key::RepoKeysSourceCreation).text(),
        KeySourceView::Creation { backfilled: true } => match date() {
            Some(d) => t
                .msg(Key::RepoKeysSourceCreationRecorded)
                .with("date", d)
                .text(),
            None => t.msg(Key::RepoKeysSourceCreation).text(),
        },
        KeySourceView::Operator { reason } => t
            .msg(Key::RepoKeysSourceOperator)
            .with("date", date().unwrap_or_default())
            .with("reason", reason)
            .text(),
        KeySourceView::Delivery { .. } => t
            .msg(Key::RepoKeysSourceDelivery)
            .with("date", date().unwrap_or_default())
            .text(),
        KeySourceView::NotRecorded => t.msg(Key::RepoKeysSourceNotRecorded).text(),
    }
}

/// The section for a page; `None` where nothing is to be said (the server has no reader).
pub(crate) fn block(read: &KeysRead, t: &T) -> Option<KeysBlock> {
    match read {
        KeysRead::Off => None,
        KeysRead::Unavailable => Some(KeysBlock {
            heading: String::new(),
            count: None,
            note: Some(t.msg(Key::RepoKeysUnavailable).text()),
            items: Vec::new(),
        }),
        KeysRead::Keys(keys) if keys.is_empty() => Some(KeysBlock {
            heading: String::new(),
            count: None,
            note: Some(t.msg(Key::RepoKeysNone).text()),
            items: Vec::new(),
        }),
        KeysRead::Keys(keys) => Some(KeysBlock {
            // "Keys this repository trusts" (the message that was already signed off) and, beside it, the number
            heading: t.get(Key::RepoTrustedKeys).to_owned(),
            count: Some(t.msg(Key::RepoKeysCount).with("total", keys.len()).text()),
            note: None,
            items: keys
                .iter()
                .map(|k| KeyItem {
                    key_id: k.key_id.clone(),
                    fingerprint: k.fingerprint.clone(),
                    source: source_sentence(t, k),
                })
                .collect(),
        }),
    }
}

// ------------------------------------------------------------------------------------------------ the JSON

/// `source` of a key in the JSON. Like every view of the API it omits no field: one that does not apply to the kind is `null`.
#[derive(Debug, Serialize)]
pub struct SourceJson {
    /// `creation`, `delivery`, `operator` or `not_recorded`.
    pub kind: &'static str,
    /// For `creation`: the record was written later, when it was first filled for a repository that already existed.
    pub backfilled: Option<bool>,
    /// Unix seconds when the record was written; absent for `not_recorded`.
    pub recorded_at: Option<u64>,
    pub delivery_id: Option<String>,
    /// For `operator`: the operator's reason (display-safe text, at most 200 characters).
    pub reason: Option<String>,
}

/// One trusted key in the JSON.
#[derive(Debug, Serialize)]
pub struct TrustedKeyJson {
    pub key_id: String,
    pub public_key: String,
    pub fingerprint: String,
    pub source: SourceJson,
}

/// The JSON of the keys, in the page's order; `None` when they cannot be read now (or the server has no reader): the field is `null`.
pub(crate) fn json_of(read: &KeysRead) -> Option<Vec<TrustedKeyJson>> {
    let KeysRead::Keys(keys) = read else {
        return None;
    };
    Some(
        keys.iter()
            .map(|k| {
                let mut source = SourceJson {
                    kind: "not_recorded",
                    backfilled: None,
                    recorded_at: k.recorded_at,
                    delivery_id: None,
                    reason: None,
                };
                match &k.source {
                    KeySourceView::Creation { backfilled } => {
                        source.kind = "creation";
                        source.backfilled = Some(*backfilled);
                    }
                    KeySourceView::Delivery { delivery_id } => {
                        source.kind = "delivery";
                        source.delivery_id = delivery_id.clone();
                    }
                    KeySourceView::Operator { reason } => {
                        source.kind = "operator";
                        source.reason = Some(reason.clone());
                    }
                    KeySourceView::NotRecorded => {}
                }
                TrustedKeyJson {
                    key_id: k.key_id.clone(),
                    public_key: k.public_key.clone(),
                    fingerprint: k.fingerprint.clone(),
                    source,
                }
            })
            .collect(),
    )
}
