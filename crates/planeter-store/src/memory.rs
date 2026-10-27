//! An in-memory [`RepositoryStore`] — a real, thread-safe backend for tests and development. The
//! production SQLite backend (RFC 001 IQ-4) is a later increment behind the same trait.

use std::collections::{HashMap, HashSet};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::model::{Owner, ProposalPolicy, RepoId, RepositoryRecord};
use crate::store::{RepositoryStore, Result, StoreError};

#[derive(Default)]
struct Inner {
    by_id: HashMap<RepoId, RepositoryRecord>,
    by_owner_name: HashMap<(Owner, String), RepoId>,
    /// The ids in the state `creating` (invisible to every read but [`RepositoryStore::creating`]).
    creating: HashSet<RepoId>,
}

/// A `RwLock`-guarded in-memory store.
#[derive(Default)]
pub struct InMemoryRepositoryStore {
    inner: RwLock<Inner>,
}

impl InMemoryRepositoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, Inner>> {
        self.inner
            .read()
            .map_err(|_| StoreError::Backend("store lock poisoned".to_owned()))
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Inner>> {
        self.inner
            .write()
            .map_err(|_| StoreError::Backend("store lock poisoned".to_owned()))
    }
}

impl InMemoryRepositoryStore {
    fn insert(&self, record: RepositoryRecord, creating: bool) -> Result<()> {
        let mut inner = self.write()?;
        if inner.by_id.contains_key(&record.repo_id) {
            return Err(StoreError::Conflict(format!(
                "repo_id {} already exists",
                record.repo_id
            )));
        }
        let key = (record.owner.clone(), record.name.clone());
        if inner.by_owner_name.contains_key(&key) {
            return Err(StoreError::Conflict(format!(
                "{}/{} already exists",
                record.owner.handle(),
                record.name
            )));
        }
        inner.by_owner_name.insert(key, record.repo_id.clone());
        if creating {
            inner.creating.insert(record.repo_id.clone());
        }
        inner.by_id.insert(record.repo_id.clone(), record);
        Ok(())
    }
}

impl RepositoryStore for InMemoryRepositoryStore {
    fn create(&self, record: RepositoryRecord) -> Result<()> {
        self.insert(record, false)
    }

    fn get(&self, repo_id: &RepoId) -> Result<Option<RepositoryRecord>> {
        let inner = self.read()?;
        if inner.creating.contains(repo_id) {
            return Ok(None);
        }
        Ok(inner.by_id.get(repo_id).cloned())
    }

    fn resolve(&self, owner: &Owner, name: &str) -> Result<Option<RepoId>> {
        let inner = self.read()?;
        Ok(inner
            .by_owner_name
            .get(&(owner.clone(), name.to_owned()))
            .filter(|id| !inner.creating.contains(*id))
            .cloned())
    }

    fn rename(&self, repo_id: &RepoId, new_owner: Owner, new_name: String) -> Result<()> {
        let mut inner = self.write()?;
        if inner.creating.contains(repo_id) {
            return Err(StoreError::NotFound);
        }
        let old_key = {
            let record = inner.by_id.get(repo_id).ok_or(StoreError::NotFound)?;
            (record.owner.clone(), record.name.clone())
        };
        let new_key = (new_owner.clone(), new_name.clone());
        if new_key != old_key && inner.by_owner_name.contains_key(&new_key) {
            return Err(StoreError::Conflict(format!(
                "{}/{} already exists",
                new_owner.handle(),
                new_name
            )));
        }
        inner.by_owner_name.remove(&old_key);
        inner.by_owner_name.insert(new_key, repo_id.clone());
        let record = inner.by_id.get_mut(repo_id).ok_or(StoreError::NotFound)?;
        record.owner = new_owner;
        record.name = new_name;
        // repo_id and path are intentionally left unchanged (RFC 001 D-5).
        Ok(())
    }

    fn set_prikk_format_version(&self, repo_id: &RepoId, version: String) -> Result<()> {
        let mut inner = self.write()?;
        let record = inner.by_id.get_mut(repo_id).ok_or(StoreError::NotFound)?;
        record.prikk_format_version = Some(version);
        Ok(())
    }

    fn set_proposals(&self, repo_id: &RepoId, policy: ProposalPolicy) -> Result<()> {
        let mut inner = self.write()?;
        let record = inner.by_id.get_mut(repo_id).ok_or(StoreError::NotFound)?;
        record.proposals = policy;
        Ok(())
    }

    fn delete(&self, repo_id: &RepoId) -> Result<()> {
        let mut inner = self.write()?;
        let record = inner.by_id.remove(repo_id).ok_or(StoreError::NotFound)?;
        inner.creating.remove(repo_id);
        inner.by_owner_name.remove(&(record.owner, record.name));
        Ok(())
    }

    fn list_by_owner(&self, owner: &Owner) -> Result<Vec<RepositoryRecord>> {
        let inner = self.read()?;
        Ok(inner
            .by_id
            .values()
            .filter(|r| &r.owner == owner && !inner.creating.contains(&r.repo_id))
            .cloned()
            .collect())
    }

    fn reserve(&self, record: RepositoryRecord) -> Result<()> {
        self.insert(record, true)
    }

    fn mark_ready(&self, repo_id: &RepoId) -> Result<()> {
        if self.write()?.creating.remove(repo_id) {
            Ok(())
        } else {
            Err(StoreError::NotFound)
        }
    }

    fn creating(&self) -> Result<Vec<RepositoryRecord>> {
        let inner = self.read()?;
        let mut out: Vec<RepositoryRecord> = inner
            .creating
            .iter()
            .filter_map(|id| inner.by_id.get(id).cloned())
            .collect();
        out.sort_by(|a, b| a.repo_id.as_str().cmp(b.repo_id.as_str()));
        Ok(out)
    }

    fn known_ids(&self) -> Result<Vec<RepoId>> {
        Ok(self.read()?.by_id.keys().cloned().collect())
    }

    fn count_owned(&self, owner: &Owner) -> Result<u64> {
        Ok(self
            .read()?
            .by_id
            .values()
            .filter(|r| &r.owner == owner)
            .count() as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::Visibility;

    fn record(id: &str, owner: Owner, name: &str) -> RepositoryRecord {
        RepositoryRecord {
            repo_id: RepoId::new(id),
            owner,
            name: name.to_owned(),
            visibility: Visibility::Private,
            created_at: 0,
            prikk_format_version: None,
            proposals: Default::default(),
            path: PathBuf::from(format!("/repos/{id}/.prikk")),
        }
    }

    #[test]
    fn set_proposals_changes_the_setting_and_refuses_an_unknown_repository() {
        let s = InMemoryRepositoryStore::new();
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        assert_eq!(
            s.get(&RepoId::new("r1")).unwrap().unwrap().proposals,
            ProposalPolicy::Members
        );
        s.set_proposals(&RepoId::new("r1"), ProposalPolicy::AnyAuthenticated)
            .unwrap();
        assert_eq!(
            s.get(&RepoId::new("r1")).unwrap().unwrap().proposals,
            ProposalPolicy::AnyAuthenticated
        );
        assert!(matches!(
            s.set_proposals(&RepoId::new("nope"), ProposalPolicy::Members),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn create_get_resolve() {
        let s = InMemoryRepositoryStore::new();
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        let got = s.get(&RepoId::new("r1")).unwrap().unwrap();
        assert_eq!(got.name, "app");
        let id = s
            .resolve(&Owner::User("alice".into()), "app")
            .unwrap()
            .unwrap();
        assert_eq!(id, RepoId::new("r1"));
        assert!(s.get(&RepoId::new("nope")).unwrap().is_none());
    }

    #[test]
    fn duplicate_owner_name_conflicts() {
        let s = InMemoryRepositoryStore::new();
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        let err = s
            .create(record("r2", Owner::User("alice".into()), "app"))
            .unwrap_err();
        assert!(matches!(err, StoreError::Conflict(_)));
    }

    #[test]
    fn rename_and_transfer_keep_repo_id_and_path() {
        let s = InMemoryRepositoryStore::new();
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        let before = s.get(&RepoId::new("r1")).unwrap().unwrap();

        // rename + transfer to an org
        s.rename(&RepoId::new("r1"), Owner::Org("acme".into()), "core".into())
            .unwrap();
        let after = s.get(&RepoId::new("r1")).unwrap().unwrap();

        assert_eq!(after.repo_id, before.repo_id); // id unchanged
        assert_eq!(after.path, before.path); // no bytes move
        assert_eq!(after.owner, Owner::Org("acme".into()));
        assert_eq!(after.name, "core");

        // old owner/name no longer resolves; the new one does
        assert!(
            s.resolve(&Owner::User("alice".into()), "app")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            s.resolve(&Owner::Org("acme".into()), "core").unwrap(),
            Some(RepoId::new("r1"))
        );
    }

    #[test]
    fn rename_into_a_taken_name_conflicts() {
        let s = InMemoryRepositoryStore::new();
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        s.create(record("r2", Owner::User("alice".into()), "lib"))
            .unwrap();
        let err = s
            .rename(
                &RepoId::new("r2"),
                Owner::User("alice".into()),
                "app".into(),
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::Conflict(_)));
    }

    #[test]
    fn set_format_version_delete_and_list() {
        let s = InMemoryRepositoryStore::new();
        s.create(record("r1", Owner::User("alice".into()), "app"))
            .unwrap();
        s.create(record("r2", Owner::Org("acme".into()), "core"))
            .unwrap();

        s.set_prikk_format_version(&RepoId::new("r1"), "6".into())
            .unwrap();
        assert_eq!(
            s.get(&RepoId::new("r1"))
                .unwrap()
                .unwrap()
                .prikk_format_version
                .as_deref(),
            Some("6")
        );

        let alice = s.list_by_owner(&Owner::User("alice".into())).unwrap();
        assert_eq!(alice.len(), 1);

        s.delete(&RepoId::new("r1")).unwrap();
        assert!(s.get(&RepoId::new("r1")).unwrap().is_none());
        assert!(matches!(
            s.delete(&RepoId::new("r1")).unwrap_err(),
            StoreError::NotFound
        ));
    }
}
