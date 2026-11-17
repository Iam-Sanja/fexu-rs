//! Object policies. Query scopes filter list and write operations in SQL.
//! New records are checked before INSERT.

use crate::service::AuthUser;
use fexu_core::QueryScope;
use fexu_db::ModelRepository;

pub trait ObjectPolicy<T> {
    fn can_read(user: &AuthUser, object: &T) -> bool;
    fn can_update(user: &AuthUser, object: &T) -> bool;
    fn can_delete(user: &AuthUser, object: &T) -> bool;

    /// The default scope matches no rows. Policies must explicitly return `All` or `Equals`.
    fn read_scope(_user: &AuthUser) -> QueryScope {
        QueryScope::None
    }

    fn write_scope(user: &AuthUser) -> QueryScope {
        Self::read_scope(user)
    }

    /// Creation is denied by default. A create policy must explicitly reject unauthorized ownership.
    fn can_create(_user: &AuthUser, _input: &<T as ModelRepository>::Create) -> bool
    where
        T: ModelRepository,
    {
        false
    }
}
