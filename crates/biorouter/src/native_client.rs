//! Ephemeral folder-consent ownership for one authenticated native app connection.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};

use biorouter_sandbox::{bounded_fs::GrantRevocation, RootGrant};
use tokio::task_local;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

static CLIENTS: LazyLock<Mutex<HashMap<Uuid, Weak<Client>>>> = LazyLock::new(Mutex::default);

task_local! {
    static CURRENT: Option<NativeClient>;
}

#[derive(Debug)]
struct State {
    live: bool,
    roots: Vec<GrantRevocation>,
}

#[derive(Debug)]
struct Client {
    id: Uuid,
    state: Mutex<State>,
    ended: CancellationToken,
}

#[derive(Clone)]
pub struct NativeClient(Arc<Client>);

/// Owned only by the authenticated native connection, never a running tool.
pub struct NativeClientLease {
    client: NativeClient,
}

impl NativeClientLease {
    /// The caller must authenticate the main-process connection with native user proof.
    pub fn register() -> anyhow::Result<Self> {
        let client = NativeClient(Arc::new(Client {
            id: Uuid::new_v4(),
            ended: CancellationToken::new(),
            state: Mutex::new(State {
                live: true,
                roots: Vec::new(),
            }),
        }));
        let mut clients = CLIENTS.lock().unwrap_or_else(PoisonError::into_inner);
        clients.retain(|_, client| client.strong_count() > 0);
        anyhow::ensure!(
            clients.len() < 128,
            "Too many native app connections. Close an unused app and reconnect."
        );
        clients.insert(client.id(), Arc::downgrade(&client.0));
        Ok(Self { client })
    }

    pub fn client(&self) -> NativeClient {
        self.client.clone()
    }
}

impl Drop for NativeClientLease {
    fn drop(&mut self) {
        self.client.0.ended.cancel();
        let mut state = self
            .client
            .0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.live = false;
        for root in state.roots.drain(..) {
            root.revoke();
        }
        drop(state);
        CLIENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.client.id());
        crate::folder_access::FolderAccessManager::global().expire_native_client(self.client.id());
    }
}

impl std::fmt::Debug for NativeClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeClient")
            .field("live", &self.is_live())
            .finish_non_exhaustive()
    }
}

impl NativeClient {
    pub fn unavailable() -> Self {
        Self(Arc::new(Client {
            id: Uuid::nil(),
            ended: CancellationToken::new(),
            state: Mutex::new(State {
                live: false,
                roots: Vec::new(),
            }),
        }))
    }

    pub fn lookup(id: &str) -> Option<Self> {
        let id = Uuid::parse_str(id).ok()?;
        let client = CLIENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)?
            .upgrade()?;
        let client = Self(client);
        client.is_live().then_some(client)
    }

    pub async fn ended(&self) {
        self.0.ended.cancelled().await;
    }

    pub fn id(&self) -> Uuid {
        self.0.id
    }

    pub fn is_live(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .live
    }

    pub fn track(&self, root: &RootGrant) -> anyhow::Result<()> {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        anyhow::ensure!(
            state.live,
            "The desktop connection ended. Reopen this chat and approve the folder again."
        );
        state.roots.retain(GrantRevocation::is_attached);
        anyhow::ensure!(
            state.roots.len() < 4096,
            "Too many active folder operations. Finish an existing operation before retrying."
        );
        state.roots.push(root.revocation_handle());
        Ok(())
    }
}

pub fn current() -> Option<NativeClient> {
    CURRENT.try_with(Clone::clone).ok().flatten()
}

pub async fn scope<F: Future>(client: Option<NativeClient>, future: F) -> F::Output {
    CURRENT.scope(client, future).await
}

/// Capture before spawning: Tokio task locals do not propagate to child tasks.
pub fn carry<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let client = current();
    async move { scope(client, future).await }
}

pub fn same_owner(owner: Option<&NativeClient>, current: Option<&NativeClient>) -> bool {
    match (owner, current) {
        (None, None) => true,
        (Some(owner), Some(current)) => owner.id() == current.id() && owner.is_live(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter_sandbox::{
        bounded_fs::authorized_path, AccessMode, BoundedFs, DispatchAuthority,
    };

    #[tokio::test]
    async fn native_owner_revokes_retained_write_capability_and_cannot_publish_after_end() {
        let fixture = tempfile::tempdir().unwrap();
        let file = fixture.path().canonicalize().unwrap().join("synthetic.txt");
        std::fs::write(&file, "UNCHANGED").unwrap();
        let lease = NativeClientLease::register().unwrap();
        let owner = lease.client();
        let grant = RootGrant::new(
            Arc::new(BoundedFs::open(fixture.path()).unwrap()),
            AccessMode::Write,
        );
        owner.track(&grant).unwrap();
        let authority =
            DispatchAuthority::register("native-lifetime-test", "write", vec![grant.clone()])
                .unwrap();
        let bound = authorized_path(
            authority.token(),
            "native-lifetime-test",
            "write",
            &file,
            AccessMode::Write,
        )
        .unwrap();
        assert_eq!(bound.read_to_string().unwrap(), "UNCHANGED");
        drop(lease);
        assert!(bound.atomic_write(b"MUST_NOT_BE_WRITTEN").is_err());
        assert!(bound.read_to_string().is_err());
        assert!(owner.track(&grant).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "UNCHANGED");
        assert!(NativeClient::lookup(&owner.id().to_string()).is_none());
        tokio::time::timeout(std::time::Duration::from_millis(50), owner.ended())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn native_origin_survives_explicit_spawn_and_does_not_leak_outside_scope() {
        let lease = NativeClientLease::register().unwrap();
        let owner = lease.client();
        let id = owner.id();
        scope(Some(owner), async move {
            let child = tokio::spawn(carry(async { current().unwrap().id() }));
            assert_eq!(child.await.unwrap(), id);
        })
        .await;
        assert!(current().is_none());
        assert!(!same_owner(Some(&lease.client()), None));
        assert!(!same_owner(None, Some(&lease.client())));
        assert!(same_owner(None, None));
    }
}
