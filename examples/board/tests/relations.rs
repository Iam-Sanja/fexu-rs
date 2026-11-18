use fexu::core::{QueryScope, ScopeValue};
use fexu::db::ModelRepository;
use fexu::forms::{
    bind_inline_formset, bind_relation, BoundRelation, FexuForm, FormValues, InlineAction,
    InlineChange, InlineFormset, ModelFormset, RelationInput, RelationLink, SubmitMode,
};
use fexu::prelude::*;

use board::{
    prepare_database, save_project_with_tasks, Member, Project, ProjectForm, Task, TaskLineForm,
};

fn tasks_config() -> ModelFormset {
    ModelFormset {
        prefix: "tasks",
        min_rows: 1,
        max_rows: 20,
        can_delete: true,
        can_order: true,
    }
}

fn parse_task(_mode: SubmitMode, row: &FormValues) -> Result<String, fexu::forms::FormErrors> {
    TaskLineForm::from_submit(SubmitMode::Create, row).map(|form| form.title)
}

async fn exercise(db: fexu::db::driver::DatabaseConnection) {
    prepare_database(&db).await.expect("schema");
    let owner = Uuid::new_v4();
    let other = Uuid::new_v4();
    let ada = Member::create(
        &db,
        board::MemberCreate {
            name: "Ada".to_owned(),
            owner_id: owner,
        },
    )
    .await
    .expect("member");
    let grace = Member::create(
        &db,
        board::MemberCreate {
            name: "Grace".to_owned(),
            owner_id: other,
        },
    )
    .await
    .expect("foreign member");
    let allowed = [ada.id];
    let known = [ada.id, grace.id];
    let mut lead = FormValues::default();
    lead.insert("lead_id", grace.id.to_string());
    assert_eq!(
        bind_relation(
            &lead,
            RelationInput {
                name: "lead_id",
                link: RelationLink::ForeignKey,
                required: false,
                readonly: false,
                allowed: &allowed,
                known: &known,
                locked: &[],
            },
        )
        .unwrap_err()
        .for_field("lead_id"),
        Some("validation.relation_denied")
    );
    lead.insert("lead_id", Uuid::new_v4().to_string());
    assert_eq!(
        bind_relation(
            &lead,
            RelationInput {
                name: "lead_id",
                link: RelationLink::ForeignKey,
                required: false,
                readonly: false,
                allowed: &allowed,
                known: &known,
                locked: &[],
            },
        )
        .unwrap_err()
        .for_field("lead_id"),
        Some("validation.relation_missing")
    );
    lead.insert("lead_id", ada.id.to_string());
    let BoundRelation::One(Some(chosen)) = bind_relation(
        &lead,
        RelationInput {
            name: "lead_id",
            link: RelationLink::ForeignKey,
            required: true,
            readonly: false,
            allowed: &allowed,
            known: &known,
            locked: &[],
        },
    )
    .unwrap() else {
        panic!("lead");
    };
    assert_eq!(chosen, ada.id);
    let mut desk_values = FormValues::default();
    desk_values.set_all("desk_id", [ada.id.to_string(), grace.id.to_string()]);
    assert_eq!(
        bind_relation(
            &desk_values,
            RelationInput {
                name: "desk_id",
                link: RelationLink::OneToOne,
                required: false,
                readonly: false,
                allowed: &allowed,
                known: &known,
                locked: &[],
            },
        )
        .unwrap_err()
        .for_field("desk_id"),
        Some("validation.duplicate")
    );

    let mut posted = FormValues::default();
    posted.insert("title", "Bridge");
    posted.insert("owner_id", other.to_string());
    assert_eq!(
        ProjectForm::from_form(&posted)
            .unwrap_err()
            .for_field("owner_id"),
        Some("validation.unknown_field")
    );
    posted.remove("owner_id");
    let project = ProjectForm::from_form(&posted).unwrap();
    let mut rows = FormValues::default();
    rows.insert("tasks_TOTAL_FORMS", "2");
    rows.insert("tasks_0_title", "Survey");
    rows.insert("tasks_0_ORDER", "1");
    rows.insert("tasks_0_project_id", Uuid::new_v4().to_string());
    rows.insert("tasks_1_title", "Pour");
    rows.insert("tasks_1_ORDER", "2");
    let inline = InlineFormset {
        formset: tasks_config(),
        parent_field: "project_id",
        parent_id: Uuid::nil(),
        owned_ids: &[],
    };
    assert_eq!(
        bind_inline_formset(&rows, inline, SubmitMode::Create, parse_task, |_| Ok(()))
            .unwrap_err()
            .for_field("tasks_0_project_id"),
        Some("validation.relation_denied")
    );
    rows.remove("tasks_0_project_id");
    let parent = Uuid::nil();
    let inline = InlineFormset {
        parent_id: parent,
        ..inline
    };
    let bound = bind_inline_formset(&rows, inline, SubmitMode::Create, parse_task, |changes| {
        let mut seen = Vec::new();
        for change in changes {
            let title = match &change.action {
                InlineAction::Create(title) | InlineAction::Update { value: title, .. } => title,
                InlineAction::Delete { .. } => continue,
            };
            if seen.contains(&title.as_str()) {
                let mut errors = fexu::forms::FormErrors::default();
                errors.push_form("validation.duplicate");
                return Err(errors);
            }
            seen.push(title.as_str());
        }
        Ok(())
    })
    .unwrap();
    let saved = save_project_with_tasks(
        &db,
        owner,
        None,
        project.title.clone(),
        Some(chosen),
        Some(ada.id),
        bound,
    )
    .await
    .expect("save");
    let stored = Project::get_scoped(
        &db,
        saved,
        &QueryScope::Equals {
            column: "owner_id",
            value: ScopeValue::Uuid(owner),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(stored.lead_id, Some(ada.id));
    assert_eq!(stored.desk_id, Some(ada.id));
    let children = Task::list_scoped(
        &db,
        &QueryScope::Equals {
            column: "project_id",
            value: ScopeValue::Uuid(saved),
        },
    )
    .await
    .unwrap();
    assert_eq!(children.data.len(), 2);

    let foreign_owner = save_project_with_tasks(
        &db,
        other,
        Some(saved),
        "Stolen".to_owned(),
        None,
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(foreign_owner.unwrap_err(), "validation.relation_denied");
    assert_eq!(
        Project::get(&db, saved).await.unwrap().unwrap().title,
        "Bridge"
    );

    let second = save_project_with_tasks(
        &db,
        owner,
        None,
        "Other".to_owned(),
        None,
        Some(ada.id),
        vec![InlineChange {
            index: 0,
            order: Some(1),
            action: InlineAction::Create("Alone".to_owned()),
        }],
    )
    .await;
    assert!(second.is_err());
    assert!(Project::get(&db, saved).await.unwrap().is_some());
    assert_eq!(
        Task::list_scoped(
            &db,
            &QueryScope::Equals {
                column: "project_id",
                value: ScopeValue::Uuid(saved),
            },
        )
        .await
        .unwrap()
        .data
        .len(),
        2
    );

    let intruder = Task::create(
        &db,
        board::TaskCreate {
            project_id: saved,
            title: "Foreign".to_owned(),
            position: 9,
            owner_id: other,
        },
    )
    .await
    .unwrap();
    let before = Task::list_scoped(
        &db,
        &QueryScope::Equals {
            column: "project_id",
            value: ScopeValue::Uuid(saved),
        },
    )
    .await
    .unwrap()
    .data
    .len();
    let rolled = save_project_with_tasks(
        &db,
        owner,
        Some(saved),
        "Bridge".to_owned(),
        Some(ada.id),
        Some(ada.id),
        vec![
            InlineChange {
                index: 0,
                order: Some(3),
                action: InlineAction::Create("Extra".to_owned()),
            },
            InlineChange {
                index: 1,
                order: None,
                action: InlineAction::Update {
                    id: intruder.id,
                    value: "Rewritten".to_owned(),
                },
            },
        ],
    )
    .await;
    assert_eq!(rolled.unwrap_err(), "validation.relation_denied");
    let after = Task::list_scoped(
        &db,
        &QueryScope::Equals {
            column: "project_id",
            value: ScopeValue::Uuid(saved),
        },
    )
    .await
    .unwrap();
    assert_eq!(after.data.len(), before);
    assert!(after.data.iter().all(|task| task.title != "Extra"));
    assert_eq!(
        Task::get(&db, intruder.id).await.unwrap().unwrap().title,
        "Foreign"
    );
}

#[tokio::test]
async fn sqlite_project_formset_keeps_relations_inside_one_transaction() {
    let db = fexu::db::connect("sqlite::memory:").await.expect("sqlite");
    exercise(db).await;
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, CREATEDB, and an approved PostgreSQL test database"]
async fn postgres_project_formset_keeps_relations_inside_one_transaction() {
    let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let disposable = fexu::db::DisposablePostgres::create(&admin)
        .await
        .expect("isolated postgres");
    let db = fexu::db::connect(disposable.url()).await.expect("postgres");
    exercise(db.clone()).await;
    db.close().await.expect("close");
    disposable.drop_database().await.expect("drop");
}
