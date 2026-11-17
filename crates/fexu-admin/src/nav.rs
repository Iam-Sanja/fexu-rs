//! Seitenleiste aus registrierten Apps, erlaubten Modellen und eigenen Seiten.

use crate::http::{list_blocked, permit};
use crate::localization::catalog;
use crate::registry::AdminRegistry;
use fexu_auth::{AppState, AuthUser};
use serde_json::{json, Value};

pub(crate) struct Navigation {
    pub sections: Value,
    pub groups: Value,
}

struct Item {
    href: String,
    label: String,
    app: String,
}

pub(crate) async fn navigation(
    state: &AppState,
    user: &AuthUser,
    registry: &AdminRegistry,
    locale: &str,
    path: &str,
) -> Navigation {
    let mut models = Vec::new();
    let mut views = Vec::new();
    if let Some(auth) = state.auth.as_ref() {
        for model in registry.models() {
            if list_blocked(model.as_ref()) {
                continue;
            }
            if permit(auth, user, model.meta(), "read").await.is_err() {
                continue;
            }
            let key = model.meta().key;
            models.push(Item {
                href: format!("/admin/models/{key}"),
                label: model.meta().label.to_owned(),
                app: registry.model_app(key).unwrap_or("").to_owned(),
            });
        }
        for view in registry.views() {
            if !view.in_navigation() {
                continue;
            }
            let allowed = auth
                .holds(user, view.permission_code())
                .await
                .unwrap_or(false);
            if !allowed {
                continue;
            }
            let key = view.key();
            views.push(Item {
                href: format!("/admin/pages/{key}"),
                label: view.title(locale).to_owned(),
                app: registry.view_app(key).unwrap_or("").to_owned(),
            });
        }
    }
    let manage_roles = match state.auth.as_ref() {
        Some(auth) => auth.holds(user, "roles.update").await.unwrap_or(false),
        None => false,
    };
    let listed = crate::boards::list_boards(&state.db).await;
    let (boards, boards_error) = match listed {
        Ok(rows) => (rows, false),
        Err(_) => (Vec::new(), true),
    };
    compose(Sidebar {
        locale,
        path,
        superuser: manage_roles,
        host_label: registry.branding().app_name.as_str(),
        apps: registry.app_catalog(),
        models: &models,
        views: &views,
        boards: &boards
            .iter()
            .map(|board| (board.id.to_string(), board.name.clone()))
            .collect::<Vec<_>>(),
        boards_error,
    })
}

struct Sidebar<'a> {
    locale: &'a str,
    path: &'a str,
    superuser: bool,
    host_label: &'a str,
    apps: &'a [(String, String)],
    models: &'a [Item],
    views: &'a [Item],
    boards: &'a [(String, String)],
    boards_error: bool,
}

fn compose(sidebar: Sidebar<'_>) -> Navigation {
    let Sidebar {
        locale,
        path,
        superuser,
        host_label,
        apps,
        models,
        views,
        boards,
        boards_error,
    } = sidebar;
    let text = |key: &str| {
        catalog(locale)
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_owned()
    };
    let mut groups = Vec::new();
    for (key, label) in apps {
        push_group(&mut groups, key, label, models, views, path);
    }
    push_group(&mut groups, "", host_label, models, views, path);
    let model_groups: Vec<Value> = groups
        .iter()
        .filter(|group| {
            group["models"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        })
        .cloned()
        .collect();
    let mut dashboard_items = Vec::new();
    for (id, name) in boards {
        dashboard_items.push(link(&format!("/admin/boards/{id}"), name, path));
    }
    dashboard_items.push(link("/admin/boards/config", &text("board_config"), path));
    let mut sections = vec![json!({
        "key": "general",
        "label": text("general"),
        "items": [],
        "groups": [{
            "key": "dashboards",
            "label": text("dashboards"),
            "open": path == "/admin" || path == "/admin/" || path.starts_with("/admin/boards"),
            "error": boards_error,
            "items": dashboard_items
        }]
    })];
    sections.push(json!({
        "key": "apps",
        "label": text("applications"),
        "empty": groups.is_empty(),
        "groups": groups,
        "items": []
    }));
    if superuser {
        sections.push(json!({
            "key": "access",
            "label": text("access"),
            "groups": [],
            "items": [
                {
                    "href": "/admin/directory/users",
                    "label": text("users"),
                    "current": path.starts_with("/admin/directory/users")
                },
                {
                    "href": "/admin/directory/roles",
                    "label": text("roles"),
                    "current": path.starts_with("/admin/directory/roles")
                        || path.starts_with("/admin/directory/permissions")
                }
            ]
        }));
    }
    sections.push(json!({
        "key": "system",
        "label": text("system"),
        "groups": [],
        "items": [{
            "href": "/admin/audit",
            "label": text("history"),
            "current": path.starts_with("/admin/audit")
        }]
    }));
    Navigation {
        sections: Value::Array(sections),
        groups: Value::Array(model_groups),
    }
}

fn push_group(
    groups: &mut Vec<Value>,
    key: &str,
    label: &str,
    models: &[Item],
    views: &[Item],
    path: &str,
) {
    let mut items = Vec::new();
    let mut model_rows = Vec::new();
    for item in models.iter().filter(|item| item.app == key) {
        items.push(link(&item.href, &item.label, path));
        let model_key = item.href.trim_start_matches("/admin/models/");
        model_rows.push(json!({ "key": model_key, "label": item.label }));
    }
    for item in views.iter().filter(|item| item.app == key) {
        items.push(link(&item.href, &item.label, path));
    }
    if items.is_empty() {
        return;
    }
    let open = items.iter().any(|item| item["current"] == true);
    groups.push(json!({
        "key": key,
        "label": label,
        "open": open,
        "error": false,
        "items": items,
        "models": model_rows
    }));
}

fn link(href: &str, label: &str, path: &str) -> Value {
    json!({
        "href": href,
        "label": label,
        "current": path == href || path.starts_with(&format!("{href}/"))
    })
}

#[cfg(test)]
mod tests {
    use super::{compose, Item};

    fn item(href: &str, label: &str, app: &str) -> Item {
        Item {
            href: href.to_owned(),
            label: label.to_owned(),
            app: app.to_owned(),
        }
    }

    #[test]
    fn navigation_groups_apps_and_keeps_audit_out_of_them() {
        let apps = vec![
            ("shop".to_owned(), "Shop".to_owned()),
            ("notes".to_owned(), "Notizen".to_owned()),
        ];
        let models = vec![
            item("/admin/models/product", "Produkte", "shop"),
            item("/admin/models/note", "Notizen", "notes"),
            item("/admin/models/loose", "Lose", ""),
        ];
        let views = vec![item("/admin/pages/stock", "Bestand", "shop")];
        let boards = vec![("board-1".to_owned(), "Menü 1".to_owned())];
        let nav = compose(super::Sidebar {
            locale: "de",
            path: "/admin/models/product",
            superuser: false,
            host_label: "Fexu",
            apps: &apps,
            models: &models,
            views: &views,
            boards: &boards,
            boards_error: false,
        });
        let body = nav.sections.to_string();
        assert!(body.contains("\"label\":\"Shop\""));
        assert!(body.contains("\"label\":\"Notizen\""));
        assert!(body.contains("\"label\":\"Fexu\""));
        assert!(body.contains("\"href\":\"/admin/models/product\""));
        assert!(body.contains("\"href\":\"/admin/pages/stock\""));
        assert!(body.contains("\"current\":true"));
        assert!(body.contains("\"href\":\"/admin/audit\""));
        assert!(body.contains("\"label\":\"Dashboards\""));
        assert!(!body.contains("\"label\":\"Übersicht\""));
        assert!(body.contains("\"label\":\"Menü 1\""));
        assert!(body.contains("\"href\":\"/admin/boards/config\""));
        assert!(body.contains("\"open\":true"));
        assert!(!body.contains("\"href\":\"/admin/models\""));
        assert!(!body.contains("Benutzer und Zugang"));
        let groups = nav.groups.to_string();
        assert!(groups.contains("\"key\":\"product\""));
        assert!(!groups.contains("\"key\":\"stock\""));
        let root = compose(super::Sidebar {
            locale: "en",
            path: "/admin/audit",
            superuser: true,
            host_label: "Fexu",
            apps: &[],
            models: &[],
            views: &[],
            boards: &[],
            boards_error: true,
        });
        let root_body = root.sections.to_string();
        assert!(root_body.contains("Users and access"));
        assert!(root_body.contains("\"empty\":true"));
        assert!(root_body.contains("\"href\":\"/admin/audit\""));
        assert!(root_body.contains("\"current\":true"));
        assert!(root_body.contains("\"error\":true"));
        assert!(root_body.contains("Configuration"));
    }
}
