use factory_core::attachments::AttachmentError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

const MAX_USERS: usize = 256;

/// At most `permits` concurrent operations per user. Entries live only while an
/// operation holds or waits on its user's semaphore, and the user count is bounded.
pub(super) struct Gates {
    permits: usize,
    users: Mutex<HashMap<Uuid, Weak<Semaphore>>>,
}

impl Gates {
    pub fn new(permits: usize) -> Self {
        Self {
            permits,
            users: Mutex::default(),
        }
    }

    fn semaphore(&self, user: Uuid) -> Result<Arc<Semaphore>, AttachmentError> {
        let mut users = self
            .users
            .lock()
            .map_err(|_| AttachmentError::Unavailable)?;
        users.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = users.get(&user).and_then(Weak::upgrade) {
            return Ok(gate);
        }
        if users.len() >= MAX_USERS {
            return Err(AttachmentError::Unavailable);
        }
        let gate = Arc::new(Semaphore::new(self.permits));
        users.insert(user, Arc::downgrade(&gate));
        Ok(gate)
    }

    /// Fail instead of queueing, for long transfers.
    pub fn try_acquire(&self, user: Uuid) -> Result<OwnedSemaphorePermit, AttachmentError> {
        self.semaphore(user)?
            .try_acquire_owned()
            .map_err(|_| AttachmentError::Unavailable)
    }

    pub async fn acquire(&self, user: Uuid) -> Result<OwnedSemaphorePermit, AttachmentError> {
        self.semaphore(user)?
            .acquire_owned()
            .await
            .map_err(|_| AttachmentError::Unavailable)
    }

    #[cfg(test)]
    pub fn tracked_users(&self) -> usize {
        self.users.lock().unwrap().len()
    }
}
