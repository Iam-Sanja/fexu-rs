//! Driver errors. Constraint classification stays here so callers do not import SQLx.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    Unique,
    ForeignKey,
}

#[derive(Debug, Clone)]
pub struct RuntimeErr {
    message: String,
    constraint: Option<ConstraintKind>,
    sql_state: Option<String>,
}

impl RuntimeErr {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            constraint: None,
            sql_state: None,
        }
    }

    pub fn with_constraint(message: impl Into<String>, constraint: Option<ConstraintKind>) -> Self {
        Self {
            message: message.into(),
            constraint,
            sql_state: None,
        }
    }

    pub(crate) fn with_sql_state(
        message: impl Into<String>,
        constraint: Option<ConstraintKind>,
        sql_state: Option<String>,
    ) -> Self {
        Self {
            message: message.into(),
            constraint,
            sql_state,
        }
    }

    pub(crate) fn sql_state(&self) -> Option<&str> {
        self.sql_state.as_deref()
    }
}

impl fmt::Display for RuntimeErr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeErr {}

#[derive(Debug, thiserror::Error)]
pub enum DbErr {
    #[error("{0}")]
    Conn(RuntimeErr),
    #[error("{0}")]
    Exec(RuntimeErr),
    #[error("{0}")]
    Query(RuntimeErr),
}

impl DbErr {
    pub fn constraint(&self) -> Option<ConstraintKind> {
        match self {
            Self::Conn(runtime) | Self::Exec(runtime) | Self::Query(runtime) => runtime.constraint,
        }
    }

    pub(crate) fn sql_state(&self) -> Option<&str> {
        match self {
            Self::Conn(runtime) | Self::Exec(runtime) | Self::Query(runtime) => runtime.sql_state(),
        }
    }

    pub(crate) fn query(message: impl Into<String>) -> Self {
        Self::Query(RuntimeErr::internal(message))
    }

    pub(crate) fn exec(message: impl Into<String>) -> Self {
        Self::Exec(RuntimeErr::internal(message))
    }
}
