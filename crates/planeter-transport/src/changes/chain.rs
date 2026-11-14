//! The chain of a revision's claims (RFC 005 part 6): from the revision's `claim_ids` and `sync pending --format json`, the **foot** (the
//! claim whose block no other claim of the revision builds on), the **root** (the claim whose parent is not the block of another) and
//! the real parent of the root. Exactly one chain is supported; anything else is said, never guessed.

use std::collections::{BTreeMap, BTreeSet};

use planeter_prikk::exchange::SyncPending;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    /// A claim of the revision is no longer pending here (its work arrived in a received history, for one), or the revision has none.
    ClaimNotHeld,
    /// Several heads, an unconnected set, a merge, or two claims for one block.
    NotOneChange,
}

/// The one chain of a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// The claim to extract (`extract --claim`): the newest of the chain.
    pub foot_claim: String,
    /// The block the foot claim seals.
    pub foot_target: String,
    /// The real parent of the oldest claim: the block the "before" tree is. `None`: the revision starts a history.
    pub root_parent: Option<String>,
    pub claims: usize,
}

pub fn build_chain(claim_ids: &[String], pending: &SyncPending) -> Result<Chain, ChainError> {
    // every distinct claim the server holds, by id
    let mut held: BTreeMap<&str, (&str, &[String])> = BTreeMap::new();
    for p in &pending.patches {
        for c in &p.claims {
            held.entry(c.claim_id.as_str())
                .or_insert((c.target_block_id.as_str(), c.parent_block_ids.as_slice()));
        }
    }
    if claim_ids.is_empty() {
        return Err(ChainError::ClaimNotHeld);
    }
    let mut mine: BTreeMap<&str, (&str, &[String])> = BTreeMap::new();
    for id in claim_ids {
        match held.get(id.as_str()) {
            Some(v) => {
                mine.insert(id.as_str(), *v);
            }
            None => return Err(ChainError::ClaimNotHeld),
        }
    }
    if mine.values().any(|(_, parents)| parents.len() > 1) {
        return Err(ChainError::NotOneChange);
    }
    let mut by_target: BTreeMap<&str, &str> = BTreeMap::new();
    for (id, (target, _)) in &mine {
        if by_target.insert(target, id).is_some() {
            return Err(ChainError::NotOneChange);
        }
    }
    let parents_of_mine: BTreeSet<&str> = mine
        .values()
        .filter_map(|(_, parents)| parents.first().map(String::as_str))
        .collect();
    let feet: Vec<&str> = mine
        .iter()
        .filter(|(_, (target, _))| !parents_of_mine.contains(target))
        .map(|(id, _)| *id)
        .collect();
    let [foot] = feet.as_slice() else {
        return Err(ChainError::NotOneChange);
    };
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cur = *foot;
    let root_parent = loop {
        if !seen.insert(cur) {
            return Err(ChainError::NotOneChange);
        }
        let (_, parents) = mine[cur];
        match parents.first() {
            None => break None,
            Some(parent) => match by_target.get(parent.as_str()) {
                Some(next) => cur = next,
                None => break Some(parent.clone()),
            },
        }
    };
    if seen.len() != mine.len() {
        return Err(ChainError::NotOneChange);
    }
    Ok(Chain {
        foot_claim: (*foot).to_owned(),
        foot_target: mine[*foot].0.to_owned(),
        root_parent,
        claims: mine.len(),
    })
}
