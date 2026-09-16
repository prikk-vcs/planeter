//! An in-memory [`RepositoryStore`] — a real, thread-safe backend for tests and development. The
//! production SQLite backend (RFC 001 IQ-4) is a later increment behind the same trait.

use std::collections::HashMap;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::model::{Owner, RepoId, RepositoryRecord};
use crate::store::{RepositoryStore, Result, StoreError};

#[derive(Default)]
struct Inner {
    by_id: HashMap<RepoId, RepositoryRecord>,
    by_owner_name: HashMap<(Owner, String), RepoId>,
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

impl RepositoryStore for InMemoryRepositoryStore {
    fn create(&self, record: RepositoryRecord) -> Result<()> {
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
        inner.by_id.insert(record.repo_id.clone(), record);
        Ok(())
    }

    fn get(&self, repo_id: &RepoId) -> Result<Option<RepositoryRecord>> {
        Ok(self.read()?.by_id.get(repo_id).cloned())
    }

    fn resolve(&self, owner: &Owner, name: &str) -> Result<Option<RepoId>> {
        Ok(self
            .read()?
            .by_owner_name
            .get(&(owner.clone(), name.to_owned()))
            .cloned())
    }

    fn rename(&self, repo_id: &RepoId, new_owner: Owner, new_name: String) -> Result<()> {
        let mut inner = self.write()?;
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

    fn delete(&self, repo_id: &RepoId) -> Result<()> {
        let mut inner = self.write()?;
        let record = inner.by_id.remove(repo_id).ok_or(StoreError::NotFound)?;
        inner.by_owner_name.remove(&(record.owner, record.name));
        Ok(())
    }

    fn list_by_owner(&self, owner: &Owner) -> Result<Vec<RepositoryRecord>> {
        Ok(self
            .read()?
            .by_id
            .values()
            .filter(|r| &r.owner == owner)
            .cloned()
            .collect())
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
            path: PathBuf::from(format!("/repos/{id}/.prikk")),
        }
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
