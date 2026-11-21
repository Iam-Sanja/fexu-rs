//! Local admin used by the browser acceptance tests.
//!
//! The process binds a loopback address, creates one SQLite file, and seeds a
//! staff account that is not a superuser. myshop stays unchanged.

use fexu::admin::{AdminRegistry, ModelAdmin};
use fexu::auth::{AuthService, AuthUser, ObjectPolicy};
use fexu::core::{FexuModel, QueryScope, ScopeValue};
use fexu::db::ModelRepository;
use fexu::prelude::*;
use fexu::sea_orm::{
    sea_query::{Alias, ColumnDef, Index, PostgresQueryBuilder, SqliteQueryBuilder, Table},
    ConnectionTrait, DatabaseBackend, Statement,
};
use fexu::App;

const STAFF_EMAIL: &str = "staff@example.test";
const READER_EMAIL: &str = "reader@example.test";
const OTHER_EMAIL: &str = "other@example.test";

fn browser_password() -> String {
    let password = std::env::var("FEXU_BROWSER_ADMIN_PASSWORD")
        .unwrap_or_else(|_| panic!("FEXU_BROWSER_ADMIN_PASSWORD is required"));
    if password.len() < 12 {
        panic!("FEXU_BROWSER_ADMIN_PASSWORD is too short");
    }
    password
}

#[derive(FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "shelves",
    api = "/api/v1/shelves",
    permissions = "shelves",
    object_policy = true,
    admin = true,
    admin_label = "Shelves",
    admin_permission = "shelves",
    admin_per_page = 25
)]
struct Shelf {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required)]
    owner: String,
    #[field(required, min_length = 1, max_length = 100)]
    #[admin(list_display, searchable, sortable, label = "Name")]
    name: String,
}

struct ShelfPolicy;

impl ObjectPolicy<Shelf> for ShelfPolicy {
    fn can_read(user: &AuthUser, object: &Shelf) -> bool {
        object.owner == user.id.to_string()
    }
    fn can_update(user: &AuthUser, object: &Shelf) -> bool {
        object.owner == user.id.to_string()
    }
    fn can_delete(user: &AuthUser, object: &Shelf) -> bool {
        object.owner == user.id.to_string()
    }
    fn read_scope(user: &AuthUser) -> QueryScope {
        QueryScope::Equals {
            column: "owner",
            value: ScopeValue::Text(user.id.to_string()),
        }
    }
    fn write_scope(user: &AuthUser) -> QueryScope {
        Self::read_scope(user)
    }
    fn can_create(user: &AuthUser, input: &ShelfCreate) -> bool {
        input.owner == user.id.to_string()
    }
}

#[derive(FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "desks",
    api = "/api/v1/desks",
    admin = true,
    admin_label = "Desks",
    admin_permission = "desks"
)]
struct Desk {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, unique, min_length = 1, max_length = 80)]
    #[admin(list_display, searchable, label = "Name")]
    name: String,
}

#[derive(FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "tags",
    api = "/api/v1/tags",
    admin = true,
    admin_label = "Tags",
    admin_permission = "tags"
)]
struct Tag {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    #[admin(list_display, searchable, sortable, label = "Name")]
    name: String,
}

#[derive(FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "items",
    api = "/api/v1/items",
    admin = true,
    admin_label = "Items",
    admin_permission = "items"
)]
struct Item {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 3, max_length = 80)]
    #[admin(list_display, list_link, searchable, label = "Name")]
    name: String,
    #[field(default = true)]
    #[admin(list_display, label = "Active")]
    active: bool,
    #[field(foreign_key = "shelves.id", on_delete = "restrict")]
    #[admin(list_display, filterable, label = "Shelf")]
    shelf_id: Option<Uuid>,
    #[field(foreign_key = "desks.id", on_delete = "restrict", unique)]
    #[admin(label = "Desk")]
    desk_id: Option<Uuid>,
}

#[derive(FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "projects",
    api = "/api/v1/projects",
    admin = true,
    admin_label = "Projects",
    admin_permission = "projects",
    has_many = "tasks=Task.project_id",
    admin_inline = "tasks=tasks.project_id|form|add|change|delete|label=Tasks|max=20|extra=1|fields=label"
)]
struct Project {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    #[admin(list_display, list_link, searchable, label = "Name")]
    name: String,
}

#[derive(FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "tasks",
    api = "/api/v1/tasks",
    admin = true,
    admin_label = "Tasks",
    admin_permission = "projects"
)]
struct Task {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(foreign_key = "projects.id", on_delete = "restrict")]
    project_id: Uuid,
    #[field(required, min_length = 1, max_length = 80)]
    #[admin(list_display, label = "Label")]
    label: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:4174".to_owned());
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://admin-browser.sqlite3?mode=rwc".to_owned());
    reset_sqlite_file(&url);
    let db = fexu::db::connect(&url).await?;
    fexu::auth::apply_migrations(&db).await?;
    let mut auth_config = fexu::auth::AuthConfig::from_env();
    auth_config.apply_bound_address(&address);
    let auth = AuthService::new(db.clone(), auth_config);
    let password = browser_password();
    let staff = auth
        .create_user(STAFF_EMAIL, &password, true, false)
        .await?;
    let other = auth
        .create_user(OTHER_EMAIL, &password, true, false)
        .await?;
    auth.create_user(READER_EMAIL, &password, true, false)
        .await?;
    grant(&auth, "reader", &[("items", &["read"])], READER_EMAIL).await?;
    grant(
        &auth,
        "clerk",
        &[
            ("shelves", &["read"]),
            ("desks", &["read"]),
            ("tags", &["read"]),
            ("items", &["read", "create", "update", "delete"]),
            ("projects", &["read", "create", "update", "delete"]),
        ],
        STAFF_EMAIL,
    )
    .await?;

    let items = ModelAdmin::new("items")
        .many_to_many("tags", "Tags", "item_tags", "item_id", "tag_id", "tags")
        .check(Item::admin_meta().unwrap(), Item::columns())?;
    let registry = AdminRegistry::new()
        .add_policy::<Shelf, ShelfPolicy>()
        .add::<Desk>()
        .add::<Tag>()
        .add::<Item>()
        .add::<Project>()
        .add::<Task>()
        .configure(items)?
        .seal()?;
    let app = App::new(db.clone())
        .with_auth(auth)
        .with_admin(registry)
        .register::<Shelf>()
        .register::<Desk>()
        .register::<Tag>()
        .register::<Item>()
        .register::<Project>()
        .register::<Task>();
    app.bootstrap_schema().await?;
    create_link_table(&db).await?;

    let mut chosen_shelf = None;
    for index in 0..30 {
        let shelf = Shelf::create(
            &db,
            ShelfCreate {
                owner: staff.id.to_string(),
                name: format!("Regal {index:02}"),
            },
        )
        .await?;
        if index == 20 {
            chosen_shelf = Some(shelf.id);
        }
    }
    Shelf::create(
        &db,
        ShelfCreate {
            owner: other.id.to_string(),
            name: "Fremd-Regal".into(),
        },
    )
    .await?;
    let desk_a = Desk::create(
        &db,
        DeskCreate {
            name: "Pult A".into(),
        },
    )
    .await?;
    Desk::create(
        &db,
        DeskCreate {
            name: "Pult B".into(),
        },
    )
    .await?;
    let mut late_tag = None;
    for index in 0..101 {
        let tag = Tag::create(
            &db,
            TagCreate {
                name: format!("Tag {index:03}"),
            },
        )
        .await?;
        if index == 100 {
            late_tag = Some(tag.id);
        }
    }
    let item = Item::create(
        &db,
        ItemCreate {
            name: "Gewählt".into(),
            active: Some(true),
            shelf_id: chosen_shelf,
            desk_id: Some(desk_a.id),
        },
    )
    .await?;
    fexu::admin::replace_links(
        &db,
        fexu::db::ManyToMany {
            join_table: "item_tags",
            parent_column: "item_id",
            child_column: "tag_id",
        },
        item.id,
        &[late_tag.ok_or("the last tag was not created")?],
    )
    .await?;
    for name in ["Bulk A", "Bulk B"] {
        Item::create(
            &db,
            ItemCreate {
                name: name.into(),
                active: Some(true),
                shelf_id: None,
                desk_id: None,
            },
        )
        .await?;
    }

    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Fexu admin browser at http://{address}");
    fexu::serve(listener, app.into_router()).await?;
    Ok(())
}

async fn grant(
    auth: &AuthService,
    role: &str,
    groups: &[(&str, &[&str])],
    email: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    auth.ensure_role(role).await?;
    for (resource, actions) in groups {
        for action in *actions {
            let code = format!("{resource}.{action}");
            auth.ensure_permission(&code).await?;
            auth.grant_permission(role, &code).await?;
        }
    }
    auth.assign_role(email, role).await?;
    Ok(())
}

fn reset_sqlite_file(url: &str) {
    let Some(rest) = url.strip_prefix("sqlite:") else {
        return;
    };
    if rest.starts_with(":memory:") {
        return;
    }
    let path = rest.split('?').next().unwrap_or(rest);
    let path = if let Some(absolute) = path.strip_prefix("//") {
        absolute
    } else {
        path
    };
    if !path.is_empty() {
        let _ = std::fs::remove_file(path);
    }
}

// The fixture uses the same typed UUID links on both supported backends.
async fn create_link_table(
    db: &fexu::sea_orm::DatabaseConnection,
) -> Result<(), Box<dyn std::error::Error>> {
    let links = Table::create()
        .table(Alias::new("item_tags"))
        .col(ColumnDef::new(Alias::new("item_id")).uuid().not_null())
        .col(ColumnDef::new(Alias::new("tag_id")).uuid().not_null())
        .index(
            Index::create()
                .col(Alias::new("item_id"))
                .col(Alias::new("tag_id"))
                .unique(),
        )
        .to_owned();
    let backend = db.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Sqlite => links.to_string(SqliteQueryBuilder),
        DatabaseBackend::Postgres => links.to_string(PostgresQueryBuilder),
        _ => return Err("The browser fixture supports SQLite and PostgreSQL.".into()),
    };
    db.execute(Statement::from_string(backend, sql)).await?;
    Ok(())
}

#[cfg(test)]
mod link_schema_tests {
    use super::*;
    use fexu::sea_orm::{
        sea_query::{Expr, Query, Value},
        DatabaseConnection,
    };

    async fn verify_links(db: &DatabaseConnection) {
        create_link_table(db).await.unwrap();
        let parent = Uuid::new_v4();
        let child = Uuid::new_v4();
        fexu::admin::replace_links(
            db,
            fexu::db::ManyToMany {
                join_table: "item_tags",
                parent_column: "item_id",
                child_column: "tag_id",
            },
            parent,
            &[child],
        )
        .await
        .unwrap();
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new("item_tags"))
            .columns([Alias::new("item_id"), Alias::new("tag_id")])
            .values_panic([
                Expr::val(Value::Uuid(Some(Box::new(parent)))).into(),
                Expr::val(Value::Uuid(Some(Box::new(child)))).into(),
            ]);
        let backend = db.get_database_backend();
        let (sql, values) = match backend {
            DatabaseBackend::Postgres => insert.build(PostgresQueryBuilder),
            DatabaseBackend::Sqlite => insert.build(SqliteQueryBuilder),
            _ => unreachable!(),
        };
        assert!(db
            .execute(Statement::from_sql_and_values(backend, sql, values))
            .await
            .is_err());
        fexu::admin::replace_links(
            db,
            fexu::db::ManyToMany {
                join_table: "item_tags",
                parent_column: "item_id",
                child_column: "tag_id",
            },
            parent,
            &[],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sqlite_link_schema_supports_uuid_membership_and_uniqueness() {
        let db = fexu::db::connect("sqlite::memory:").await.unwrap();
        verify_links(&db).await;
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL, CREATEDB, and an approved PostgreSQL test database"]
    async fn postgres_link_schema_supports_uuid_membership_and_uniqueness() {
        let admin = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let disposable = fexu::db::DisposablePostgres::create(&admin)
            .await
            .expect("isolated postgres");
        let db = fexu::db::connect(disposable.url()).await.expect("postgres");
        verify_links(&db).await;
        db.close().await.expect("close");
        disposable.drop_database().await.expect("drop");
    }
}
