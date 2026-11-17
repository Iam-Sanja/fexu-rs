//! Authorization to read or write a file. Ownership does not automatically transfer to another actor.

use crate::StorageError;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAction {
    Upload,
    Download,
    Delete,
}

pub trait StorageAccess: Send + Sync {
    fn permit(&self, actor: Uuid, owner: Uuid, action: StorageAction) -> Result<(), StorageError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OwnerAccess;

impl StorageAccess for OwnerAccess {
    fn permit(&self, actor: Uuid, owner: Uuid, _action: StorageAction) -> Result<(), StorageError> {
        if actor == owner {
            Ok(())
        } else {
            Err(StorageError::Forbidden)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnerAccess, StorageAccess, StorageAction};
    use uuid::Uuid;

    #[test]
    fn another_actor_cannot_upload_or_download() {
        let actor = Uuid::new_v4();
        let owner = Uuid::new_v4();
        assert!(OwnerAccess
            .permit(owner, owner, StorageAction::Upload)
            .is_ok());
        assert!(matches!(
            OwnerAccess.permit(actor, owner, StorageAction::Download),
            Err(crate::StorageError::Forbidden)
        ));
        assert!(matches!(
            OwnerAccess.permit(actor, owner, StorageAction::Delete),
            Err(crate::StorageError::Forbidden)
        ));
    }
}
