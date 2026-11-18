//! Project, member and task models for the formset example.
//!
//! Writes go through the model repository inside one transaction. A failed
//! child row rolls the parent back. The caller still supplies the owner scope.

use fexu::core::{DatabaseKind, FexuError, FexuModel, QueryScope, ScopeValue};
use fexu::db::driver::{ConnectionTrait, DatabaseConnection, Statement, TransactionTrait};
use fexu::db::{
    conclude, kind_of, render_table_bundle, ForeignKeyClause, ModelRepository, PatchField,
    PhysicalColumn,
};
use fexu::forms::{InlineAction, InlineChange};
use fexu::prelude::*;
use fexu::serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, FexuModel, Serialize, Deserialize)]
#[fexu(table = "board_members")]
pub struct Member {
    #[field(primary_key, auto)]
    pub id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    pub name: String,
    pub owner_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, FexuModel, Serialize, Deserialize)]
#[fexu(table = "board_projects")]
pub struct Project {
    #[field(primary_key, auto)]
    pub id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    pub title: String,
    pub owner_id: Uuid,
    #[field(foreign_key = "board_members.id", on_delete = "set_null")]
    pub lead_id: Option<Uuid>,
    #[field(unique, foreign_key = "board_members.id", on_delete = "set_null")]
    pub desk_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, FexuModel, Serialize, Deserialize)]
#[fexu(table = "board_tasks")]
pub struct Task {
    #[field(primary_key, auto)]
    pub id: Uuid,
    #[field(foreign_key = "board_projects.id", on_delete = "cascade")]
    pub project_id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    pub title: String,
    pub position: i64,
    pub owner_id: Uuid,
}

pub async fn create_tables(db: &DatabaseConnection) -> Result<(), String> {
    let backend = db.get_database_backend();
    let kind = kind_of(backend).map_err(|error| error.to_string())?;
    install::<Member>(db, kind).await?;
    install::<Project>(db, kind).await?;
    install::<Task>(db, kind).await?;
    Ok(())
}

async fn install<M: FexuModel>(db: &DatabaseConnection, kind: DatabaseKind) -> Result<(), String> {
    let columns = M::columns();
    let physical: Vec<PhysicalColumn<'_>> = columns.iter().map(PhysicalColumn::from).collect();
    let table = M::descriptor().table;
    let mut unique_owned = Vec::new();
    for column in columns {
        if column.unique && !column.primary_key {
            unique_owned.push((format!("{table}_{}_key", column.name), vec![column.name]));
        }
    }
    for group in M::constraints().unique_groups {
        unique_owned.push((format!("{table}_{}_key", group.join("_")), group.to_vec()));
    }
    let unique_refs: Vec<(&str, &[&str])> = unique_owned
        .iter()
        .map(|(name, cols)| (name.as_str(), cols.as_slice()))
        .collect();
    let mut foreign_names = Vec::new();
    let mut foreign_parts = Vec::new();
    for column in columns {
        if let (Some(target), Some(target_column), Some(action)) = (
            column.foreign_table,
            column.foreign_column,
            column.on_delete,
        ) {
            foreign_names.push(format!("{table}_{}_fk", column.name));
            foreign_parts.push((column.name, target, target_column, action));
        }
    }
    let foreign_keys: Vec<ForeignKeyClause<'_>> = foreign_parts
        .iter()
        .enumerate()
        .map(
            |(index, (column, target, target_column, action))| ForeignKeyClause {
                name: &foreign_names[index],
                column,
                referenced_table: target,
                referenced_column: target_column,
                on_delete: *action,
            },
        )
        .collect();
    let backend = db.get_database_backend();
    for sql in render_table_bundle(kind, table, &physical, &unique_refs, &foreign_keys, &[]) {
        db.execute(Statement::from_string(backend, sql))
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Saves a project and its tasks. `owner` is the caller's scope, not a form field.
///
/// A task whose `owner_id` differs from `owner` is rejected even when its
/// `project_id` matches. That check runs inside the transaction.
pub async fn save_project_with_tasks(
    db: &DatabaseConnection,
    owner: Uuid,
    project_id: Option<Uuid>,
    title: String,
    lead_id: Option<Uuid>,
    desk_id: Option<Uuid>,
    tasks: Vec<InlineChange<String>>,
) -> Result<Uuid, &'static str> {
    let owner_scope = owner_scope(owner);
    if let Some(id) = project_id {
        let parent = Project::get_scoped(db, id, &owner_scope)
            .await
            .map_err(error_code)?;
        if parent.is_none() {
            return Err("validation.relation_denied");
        }
    }
    let tx = db.begin().await.map_err(|_| "form.formset")?;
    let result = write_project(&tx, owner, project_id, title, lead_id, desk_id, &tasks).await;
    conclude(tx, result, |_| "form.formset").await
}

async fn write_project(
    tx: &fexu::db::driver::DatabaseTransaction,
    owner: Uuid,
    project_id: Option<Uuid>,
    title: String,
    lead_id: Option<Uuid>,
    desk_id: Option<Uuid>,
    tasks: &[InlineChange<String>],
) -> Result<Uuid, &'static str> {
    let project = match project_id {
        None => Project::create_tx(
            tx,
            ProjectCreate {
                title,
                owner_id: owner,
                lead_id,
                desk_id,
            },
        )
        .await
        .map_err(error_code)?,
        Some(id) => {
            let scope = owner_scope(owner);
            Project::update_tx(
                tx,
                id,
                ProjectUpdate {
                    title: PatchField::Value(title),
                    owner_id: PatchField::Missing,
                    lead_id: match lead_id {
                        Some(lead) => PatchField::Value(lead),
                        None => PatchField::Null,
                    },
                    desk_id: match desk_id {
                        Some(desk) => PatchField::Value(desk),
                        None => PatchField::Null,
                    },
                },
                &scope,
            )
            .await
            .map_err(error_code)?
        }
    };
    let scope = task_scope(project.id);
    for change in tasks {
        match &change.action {
            InlineAction::Delete { id } => {
                let _current = owned_task(tx, *id, &scope, owner).await?;
                Task::delete_tx(tx, *id, &scope).await.map_err(error_code)?;
            }
            InlineAction::Update { id, value } => {
                let _current = owned_task(tx, *id, &scope, owner).await?;
                Task::update_tx(
                    tx,
                    *id,
                    TaskUpdate {
                        project_id: PatchField::Missing,
                        title: PatchField::Value(value.clone()),
                        position: PatchField::Value(change.order.unwrap_or(0)),
                        owner_id: PatchField::Missing,
                    },
                    &scope,
                )
                .await
                .map_err(error_code)?;
            }
            InlineAction::Create(value) => {
                Task::create_tx(
                    tx,
                    TaskCreate {
                        project_id: project.id,
                        title: value.clone(),
                        position: change
                            .order
                            .unwrap_or(i64::try_from(change.index).unwrap_or(0)),
                        owner_id: owner,
                    },
                )
                .await
                .map_err(error_code)?;
            }
        }
    }
    Ok(project.id)
}

async fn owned_task(
    tx: &fexu::db::driver::DatabaseTransaction,
    id: Uuid,
    scope: &QueryScope,
    owner: Uuid,
) -> Result<Task, &'static str> {
    let task = Task::get_tx(tx, id, scope)
        .await
        .map_err(error_code)?
        .ok_or("validation.relation_denied")?;
    if task.owner_id != owner {
        return Err("validation.relation_denied");
    }
    Ok(task)
}

fn owner_scope(owner: Uuid) -> QueryScope {
    QueryScope::Equals {
        column: "owner_id",
        value: ScopeValue::Uuid(owner),
    }
}

fn task_scope(project: Uuid) -> QueryScope {
    QueryScope::Equals {
        column: "project_id",
        value: ScopeValue::Uuid(project),
    }
}

fn error_code(error: FexuError) -> &'static str {
    match error {
        FexuError::NotFound | FexuError::Forbidden | FexuError::Denied(_) => {
            "validation.relation_denied"
        }
        FexuError::Validation(_) => "validation.relation_denied",
        _ => "form.formset",
    }
}
