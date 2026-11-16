//! Persisted dashboard menus. Each widget identifies its model and fields.

use fexu_core::{ColumnKind, ColumnSpec, FexuError, QueryScope, ScopeValue};
use fexu_db::driver::sea_query::{
    Alias, Expr, Order, PostgresQueryBuilder, Query, SqliteQueryBuilder,
};
use fexu_db::driver::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value as DbValue,
};
use uuid::Uuid;

const BOARDS: &str = "_fexu_admin_boards";
const WIDGETS: &str = "_fexu_admin_widgets";
const BOARD_LIMIT: i64 = 24;
const WIDGET_LIMIT: i64 = 18;
pub(crate) const CHART_LIMIT: u64 = 12;
pub(crate) const GRID_COLUMNS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Board {
    pub id: Uuid,
    pub name: String,
    pub home: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Widget {
    pub id: Uuid,
    pub kind: String,
    pub model_key: String,
    pub fields: Vec<String>,
    pub group_field: String,
    pub series_field: String,
    pub measure: String,
    pub trend_field: String,
    pub span: i64,
    pub title: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

#[derive(Clone, Copy)]
pub(crate) struct Cell {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoardReject {
    Name,
    Limit,
    Fields,
    Group,
    LineDate,
    Title,
    Kind,
    Model,
    WidgetLimit,
    Place,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChartPoint {
    pub label: String,
    pub amount: i64,
}

pub(crate) struct StackPoint {
    pub group: String,
    pub series: String,
    pub amount: i64,
}

pub(crate) async fn ensure(db: &DatabaseConnection) -> Result<(), FexuError> {
    let backend = db.get_database_backend();
    execute_sql(
        db,
        backend,
        "CREATE TABLE IF NOT EXISTS _fexu_admin_boards (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            position BIGINT NOT NULL,
            created_at BIGINT NOT NULL,
            home INTEGER NOT NULL DEFAULT 0
        )",
    )
    .await?;
    execute_sql(
        db,
        backend,
        "CREATE TABLE IF NOT EXISTS _fexu_admin_widgets (
            id TEXT PRIMARY KEY NOT NULL,
            board_id TEXT NOT NULL,
            position BIGINT NOT NULL,
            kind TEXT NOT NULL,
            model_key TEXT NOT NULL,
            fields TEXT NOT NULL,
            group_field TEXT NOT NULL,
            series_field TEXT NOT NULL DEFAULT '',
            measure TEXT NOT NULL DEFAULT '',
            trend_field TEXT NOT NULL DEFAULT '',
            span BIGINT NOT NULL DEFAULT 12,
            title TEXT NOT NULL,
            grid_x BIGINT NOT NULL DEFAULT 0,
            grid_y BIGINT NOT NULL DEFAULT 0,
            grid_w BIGINT NOT NULL DEFAULT 0,
            grid_h BIGINT NOT NULL DEFAULT 0
        )",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_boards ADD COLUMN home INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN measure TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN series_field TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN trend_field TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN span BIGINT NOT NULL DEFAULT 12",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN grid_x BIGINT NOT NULL DEFAULT 0",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN grid_y BIGINT NOT NULL DEFAULT 0",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN grid_w BIGINT NOT NULL DEFAULT 0",
    )
    .await?;
    add_column(
        db,
        backend,
        "ALTER TABLE _fexu_admin_widgets ADD COLUMN grid_h BIGINT NOT NULL DEFAULT 0",
    )
    .await
}

async fn add_column(
    db: &DatabaseConnection,
    backend: DatabaseBackend,
    sql: &str,
) -> Result<(), FexuError> {
    match execute_sql(db, backend, sql).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let text = format!("{error:?}").to_lowercase();
            if text.contains("duplicate column") || text.contains("already exists") {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

pub(crate) async fn list_boards(db: &DatabaseConnection) -> Result<Vec<Board>, FexuError> {
    ensure(db).await?;
    let backend = db.get_database_backend();
    let mut query = Query::select();
    query
        .columns([Alias::new("id"), Alias::new("name"), Alias::new("home")])
        .from(Alias::new(BOARDS))
        .order_by(Alias::new("position"), Order::Asc)
        .order_by(Alias::new("name"), Order::Asc);
    let rows = fetch(db, backend, &query).await?;
    rows.into_iter().map(|row| read_board(&row)).collect()
}

pub(crate) async fn create_board(
    db: &DatabaseConnection,
    name: &str,
) -> Result<Uuid, Result<BoardReject, FexuError>> {
    let name = clean_label(name, 48).ok_or(Ok(BoardReject::Name))?;
    ensure(db).await.map_err(Err)?;
    let count = count_rows(db, BOARDS, None).await.map_err(Err)?;
    if count >= BOARD_LIMIT {
        return Err(Ok(BoardReject::Limit));
    }
    let id = Uuid::new_v4();
    let backend = db.get_database_backend();
    let mut query = Query::insert();
    query
        .into_table(Alias::new(BOARDS))
        .columns([
            Alias::new("id"),
            Alias::new("name"),
            Alias::new("position"),
            Alias::new("created_at"),
        ])
        .values_panic([
            DbValue::from(id.to_string()).into(),
            DbValue::from(name).into(),
            DbValue::from(count).into(),
            DbValue::from(unix_now()).into(),
        ]);
    execute(db, backend, &query).await.map_err(Err)?;
    Ok(id)
}

pub(crate) async fn delete_board(db: &DatabaseConnection, id: Uuid) -> Result<(), FexuError> {
    ensure(db).await?;
    let backend = db.get_database_backend();
    let mut widgets = Query::delete();
    widgets
        .from_table(Alias::new(WIDGETS))
        .and_where(Expr::col(Alias::new("board_id")).eq(id.to_string()));
    execute(db, backend, &widgets).await?;
    let mut boards = Query::delete();
    boards
        .from_table(Alias::new(BOARDS))
        .and_where(Expr::col(Alias::new("id")).eq(id.to_string()));
    execute(db, backend, &boards).await
}

pub(crate) async fn load(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<Option<(Board, Vec<Widget>)>, FexuError> {
    ensure(db).await?;
    let backend = db.get_database_backend();
    let mut board_query = Query::select();
    board_query
        .columns([Alias::new("id"), Alias::new("name"), Alias::new("home")])
        .from(Alias::new(BOARDS))
        .and_where(Expr::col(Alias::new("id")).eq(id.to_string()));
    let rows = fetch(db, backend, &board_query).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let board = read_board(&row)?;
    let mut widget_query = Query::select();
    widget_query
        .columns([
            Alias::new("id"),
            Alias::new("kind"),
            Alias::new("model_key"),
            Alias::new("fields"),
            Alias::new("group_field"),
            Alias::new("series_field"),
            Alias::new("measure"),
            Alias::new("trend_field"),
            Alias::new("span"),
            Alias::new("title"),
            Alias::new("grid_x"),
            Alias::new("grid_y"),
            Alias::new("grid_w"),
            Alias::new("grid_h"),
        ])
        .from(Alias::new(WIDGETS))
        .and_where(Expr::col(Alias::new("board_id")).eq(id.to_string()))
        .order_by(Alias::new("position"), Order::Asc);
    let widgets = fetch(db, backend, &widget_query).await?;
    let mut widgets = widgets
        .iter()
        .map(read_widget)
        .collect::<Result<Vec<_>, _>>()?;
    settle_layout(db, &mut widgets).await?;
    Ok(Some((board, widgets)))
}

pub(crate) struct NewWidget<'a> {
    pub kind: &'a str,
    pub model_key: &'a str,
    pub fields: &'a [String],
    pub group_field: &'a str,
    pub series_field: &'a str,
    pub measure: &'a str,
    pub trend_field: &'a str,
    pub span: i64,
    pub title: &'a str,
    pub group_is_date: bool,
    pub trend_is_date: bool,
}

pub(crate) async fn add_widget(
    db: &DatabaseConnection,
    board: Uuid,
    widget: NewWidget<'_>,
) -> Result<(), Result<BoardReject, FexuError>> {
    let model_key = widget.model_key.to_owned();
    let kind = accepted_kind(widget.kind)
        .ok_or(Ok(BoardReject::Kind))?
        .to_owned();
    let prepared = prepare_widget(widget, &kind)?;
    let title = prepared.title.clone();
    let fields = prepared.fields.clone();
    ensure(db).await.map_err(Err)?;
    let Some((_, existing)) = load(db, board).await.map_err(Err)? else {
        return Err(Ok(BoardReject::Model));
    };
    let count = existing.len() as i64;
    if count >= WIDGET_LIMIT {
        return Err(Ok(BoardReject::WidgetLimit));
    }
    let taken: Vec<Cell> = existing.iter().map(cell_of).collect();
    let (width, height) = footprint(&kind);
    let cell = first_slot(&taken, width, height);
    let backend = db.get_database_backend();
    let mut query = Query::insert();
    query
        .into_table(Alias::new(WIDGETS))
        .columns([
            Alias::new("id"),
            Alias::new("board_id"),
            Alias::new("position"),
            Alias::new("kind"),
            Alias::new("model_key"),
            Alias::new("fields"),
            Alias::new("group_field"),
            Alias::new("series_field"),
            Alias::new("measure"),
            Alias::new("trend_field"),
            Alias::new("span"),
            Alias::new("title"),
            Alias::new("grid_x"),
            Alias::new("grid_y"),
            Alias::new("grid_w"),
            Alias::new("grid_h"),
        ])
        .values_panic([
            DbValue::from(Uuid::new_v4().to_string()).into(),
            DbValue::from(board.to_string()).into(),
            DbValue::from(count).into(),
            DbValue::from(kind).into(),
            DbValue::from(model_key).into(),
            DbValue::from(fields).into(),
            DbValue::from(prepared.group.as_str()).into(),
            DbValue::from(prepared.series.as_str()).into(),
            DbValue::from(prepared.measure.as_str()).into(),
            DbValue::from(prepared.trend.as_str()).into(),
            DbValue::from(prepared.span).into(),
            DbValue::from(title).into(),
            DbValue::from(cell.x).into(),
            DbValue::from(cell.y).into(),
            DbValue::from(cell.w).into(),
            DbValue::from(cell.h).into(),
        ]);
    execute(db, backend, &query).await.map_err(Err)
}

pub(crate) async fn delete_widget(
    db: &DatabaseConnection,
    board: Uuid,
    widget: Uuid,
) -> Result<(), FexuError> {
    ensure(db).await?;
    let backend = db.get_database_backend();
    let mut query = Query::delete();
    query
        .from_table(Alias::new(WIDGETS))
        .and_where(Expr::col(Alias::new("id")).eq(widget.to_string()))
        .and_where(Expr::col(Alias::new("board_id")).eq(board.to_string()));
    execute(db, backend, &query).await
}

pub(crate) async fn update_widget(
    db: &DatabaseConnection,
    board: Uuid,
    widget_id: Uuid,
    widget: NewWidget<'_>,
    geometry: Option<(i64, i64, i64, i64)>,
) -> Result<(), Result<BoardReject, FexuError>> {
    let model_key = widget.model_key.to_owned();
    let kind = accepted_kind(widget.kind)
        .ok_or(Ok(BoardReject::Kind))?
        .to_owned();
    let prepared = prepare_widget(widget, &kind)?;
    let cell = if let Some((x, y, w, h)) = geometry {
        let cell = clamp_cell(&kind, x, y, w, h);
        let Some((_, widgets)) = load(db, board).await.map_err(Err)? else {
            return Err(Ok(BoardReject::Model));
        };
        if widgets
            .iter()
            .any(|item| item.id != widget_id && cell_of(item).covers(cell))
        {
            return Err(Ok(BoardReject::Place));
        }
        Some(cell)
    } else {
        None
    };
    ensure(db).await.map_err(Err)?;
    let backend = db.get_database_backend();
    let mut query = Query::update();
    let mut values = vec![
        (Alias::new("kind"), DbValue::from(kind).into()),
        (
            Alias::new("fields"),
            DbValue::from(prepared.fields.as_str()).into(),
        ),
        (
            Alias::new("group_field"),
            DbValue::from(prepared.group.as_str()).into(),
        ),
        (
            Alias::new("series_field"),
            DbValue::from(prepared.series.as_str()).into(),
        ),
        (
            Alias::new("measure"),
            DbValue::from(prepared.measure.as_str()).into(),
        ),
        (
            Alias::new("trend_field"),
            DbValue::from(prepared.trend.as_str()).into(),
        ),
        (Alias::new("span"), DbValue::from(prepared.span).into()),
        (
            Alias::new("title"),
            DbValue::from(prepared.title.as_str()).into(),
        ),
    ];
    if let Some(cell) = cell {
        values.extend([
            (Alias::new("grid_x"), DbValue::from(cell.x).into()),
            (Alias::new("grid_y"), DbValue::from(cell.y).into()),
            (Alias::new("grid_w"), DbValue::from(cell.w).into()),
            (Alias::new("grid_h"), DbValue::from(cell.h).into()),
        ]);
    }
    query
        .table(Alias::new(WIDGETS))
        .values(values)
        .and_where(Expr::col(Alias::new("id")).eq(widget_id.to_string()))
        .and_where(Expr::col(Alias::new("board_id")).eq(board.to_string()))
        .and_where(Expr::col(Alias::new("model_key")).eq(model_key));
    execute(db, backend, &query).await.map_err(Err)
}

pub(crate) async fn place_widget(
    db: &DatabaseConnection,
    board: Uuid,
    widget: Uuid,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
) -> Result<bool, FexuError> {
    let Some((_, widgets)) = load(db, board).await? else {
        return Ok(false);
    };
    let Some(current) = widgets.iter().find(|item| item.id == widget) else {
        return Ok(false);
    };
    let cell = clamp_cell(&current.kind, x, y, w, h);
    if widgets
        .iter()
        .any(|item| item.id != widget && cell_of(item).covers(cell))
    {
        return Ok(false);
    }
    write_cell(db, widget, cell).await?;
    Ok(true)
}

pub(crate) async fn move_widget(
    db: &DatabaseConnection,
    board: Uuid,
    widget: Uuid,
    up: bool,
) -> Result<(), FexuError> {
    ensure(db).await?;
    let Some((_, widgets)) = load(db, board).await? else {
        return Ok(());
    };
    let Some(index) = widgets.iter().position(|item| item.id == widget) else {
        return Ok(());
    };
    let neighbor = if up {
        index.checked_sub(1)
    } else {
        (index + 1 < widgets.len()).then_some(index + 1)
    };
    let Some(neighbor) = neighbor else {
        return Ok(());
    };
    let backend = db.get_database_backend();
    let mut positions = Query::select();
    positions
        .columns([Alias::new("id"), Alias::new("position")])
        .from(Alias::new(WIDGETS))
        .and_where(Expr::col(Alias::new("board_id")).eq(board.to_string()));
    let rows = fetch(db, backend, &positions).await?;
    let mut placed = Vec::new();
    for row in rows {
        let id = row
            .try_get::<String>("", "id")
            .map_err(FexuError::database)?;
        let position = row
            .try_get::<i64>("", "position")
            .or_else(|_| row.try_get::<i32>("", "position").map(i64::from))
            .map_err(FexuError::database)?;
        placed.push((id, position));
    }
    let current = widgets[index].id.to_string();
    let other = widgets[neighbor].id.to_string();
    let current_position = placed
        .iter()
        .find(|(id, _)| id == &current)
        .map(|(_, position)| *position)
        .unwrap_or(index as i64);
    let other_position = placed
        .iter()
        .find(|(id, _)| id == &other)
        .map(|(_, position)| *position)
        .unwrap_or(neighbor as i64);
    let (current_position, other_position) = if current_position == other_position {
        (index as i64, neighbor as i64)
    } else {
        (other_position, current_position)
    };
    set_position(db, WIDGETS, &current, current_position).await?;
    set_position(db, WIDGETS, &other, other_position).await
}

pub(crate) async fn move_board(
    db: &DatabaseConnection,
    id: Uuid,
    up: bool,
) -> Result<(), FexuError> {
    let boards = list_boards(db).await?;
    let Some(index) = boards.iter().position(|board| board.id == id) else {
        return Ok(());
    };
    let neighbor = if up {
        index.checked_sub(1)
    } else {
        (index + 1 < boards.len()).then_some(index + 1)
    };
    let Some(neighbor) = neighbor else {
        return Ok(());
    };
    let backend = db.get_database_backend();
    let mut positions = Query::select();
    positions
        .columns([Alias::new("id"), Alias::new("position")])
        .from(Alias::new(BOARDS));
    let rows = fetch(db, backend, &positions).await?;
    let mut placed = Vec::new();
    for row in rows {
        let stored = row
            .try_get::<String>("", "id")
            .map_err(FexuError::database)?;
        let position = row
            .try_get::<i64>("", "position")
            .or_else(|_| row.try_get::<i32>("", "position").map(i64::from))
            .map_err(FexuError::database)?;
        placed.push((stored, position));
    }
    let current = boards[index].id.to_string();
    let other = boards[neighbor].id.to_string();
    let current_position = placed
        .iter()
        .find(|(stored, _)| stored == &current)
        .map(|(_, position)| *position)
        .unwrap_or(index as i64);
    let other_position = placed
        .iter()
        .find(|(stored, _)| stored == &other)
        .map(|(_, position)| *position)
        .unwrap_or(neighbor as i64);
    let (current_position, other_position) = if current_position == other_position {
        (index as i64, neighbor as i64)
    } else {
        (other_position, current_position)
    };
    set_position(db, BOARDS, &current, current_position).await?;
    set_position(db, BOARDS, &other, other_position).await
}

pub(crate) async fn set_home(
    db: &DatabaseConnection,
    id: Uuid,
    enabled: bool,
) -> Result<(), FexuError> {
    ensure(db).await?;
    let backend = db.get_database_backend();
    if enabled {
        let mut clear = Query::update();
        clear
            .table(Alias::new(BOARDS))
            .values([(Alias::new("home"), DbValue::from(0_i64).into())]);
        execute(db, backend, &clear).await?;
    }
    let mut query = Query::update();
    query
        .table(Alias::new(BOARDS))
        .values([(
            Alias::new("home"),
            DbValue::from(if enabled { 1_i64 } else { 0_i64 }).into(),
        )])
        .and_where(Expr::col(Alias::new("id")).eq(id.to_string()));
    execute(db, backend, &query).await
}

pub(crate) fn materialize(widget: NewWidget<'_>) -> Result<Widget, BoardReject> {
    let model_key = widget.model_key.to_owned();
    let kind = accepted_kind(widget.kind)
        .ok_or(BoardReject::Kind)?
        .to_owned();
    let prepared = prepare_widget(widget, &kind).map_err(|error| match error {
        Ok(reject) => reject,
        Err(_) => BoardReject::Fields,
    })?;
    let fields = if prepared.fields.is_empty() {
        Vec::new()
    } else {
        prepared
            .fields
            .split(',')
            .filter(|field| !field.is_empty())
            .map(str::to_owned)
            .collect()
    };
    Ok(Widget {
        id: Uuid::nil(),
        kind,
        model_key,
        fields,
        group_field: prepared.group,
        series_field: prepared.series,
        measure: prepared.measure,
        trend_field: prepared.trend,
        span: prepared.span,
        title: prepared.title,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    })
}

pub(crate) async fn scalar(
    db: &DatabaseConnection,
    table: &str,
    columns: &[ColumnSpec],
    scope: &QueryScope,
    measure: &str,
    filters: &[(&str, DbValue)],
) -> Result<i64, FexuError> {
    if !safe_ident(table) {
        return Err(FexuError::Validation("column is not available".into()));
    }
    let backend = db.get_database_backend();
    let mut query = Query::select();
    if measure.is_empty() {
        query.expr_as(Expr::col(Alias::new("id")).count(), Alias::new("amount"));
    } else {
        if !safe_ident(measure) || !columns.iter().any(|column| column.name == measure) {
            return Err(FexuError::Validation("measure is not available".into()));
        }
        query.expr_as(Expr::col(Alias::new(measure)).sum(), Alias::new("amount"));
    }
    query.from(Alias::new(table));
    apply_filters(&mut query, columns, filters)?;
    apply_scope(&mut query, columns, scope)?;
    let rows = fetch(db, backend, &query).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(0);
    };
    Ok(amount_of(&row).unwrap_or(0))
}

pub(crate) struct AggregateQuery<'a> {
    pub db: &'a DatabaseConnection,
    pub table: &'a str,
    pub columns: &'a [ColumnSpec],
    pub scope: &'a QueryScope,
    pub filters: &'a [(&'a str, DbValue)],
}

pub(crate) async fn stacked(
    source: AggregateQuery<'_>,
    group: &str,
    series: &str,
    measure: &str,
) -> Result<Vec<StackPoint>, FexuError> {
    let AggregateQuery {
        db,
        table,
        columns,
        scope,
        filters,
    } = source;
    if group == series
        || !safe_ident(table)
        || !safe_ident(group)
        || !safe_ident(series)
        || !columns.iter().any(|column| column.name == group)
        || !columns.iter().any(|column| column.name == series)
    {
        return Err(FexuError::Validation("column is not available".into()));
    }
    let backend = db.get_database_backend();
    let mut query = Query::select();
    query.column(Alias::new(group)).column(Alias::new(series));
    if measure.is_empty() {
        query.expr_as(Expr::col(Alias::new("id")).count(), Alias::new("amount"));
    } else {
        if !safe_ident(measure) || !columns.iter().any(|column| column.name == measure) {
            return Err(FexuError::Validation("measure is not available".into()));
        }
        query.expr_as(Expr::col(Alias::new(measure)).sum(), Alias::new("amount"));
    }
    query
        .from(Alias::new(table))
        .group_by_col(Alias::new(group))
        .group_by_col(Alias::new(series))
        .order_by(Alias::new("amount"), Order::Desc)
        .limit(80);
    apply_filters(&mut query, columns, filters)?;
    apply_scope(&mut query, columns, scope)?;
    let rows = fetch(db, backend, &query).await?;
    let mut points = Vec::new();
    for row in rows {
        points.push(StackPoint {
            group: cell_label(&row, group),
            series: cell_label(&row, series),
            amount: amount_of(&row).unwrap_or(0),
        });
    }
    Ok(points)
}

struct PreparedWidget {
    title: String,
    fields: String,
    group: String,
    series: String,
    measure: String,
    trend: String,
    span: i64,
}

fn accepted_kind(kind: &str) -> Option<&str> {
    match kind {
        "table" | "bar" | "line" | "pie" | "stack" | "tree" | "card" => Some(kind),
        _ => None,
    }
}

fn prepare_widget(
    widget: NewWidget<'_>,
    kind: &str,
) -> Result<PreparedWidget, Result<BoardReject, FexuError>> {
    if widget.model_key.is_empty() || widget.model_key.len() > 80 {
        return Err(Ok(BoardReject::Model));
    }
    let needs_group = matches!(kind, "bar" | "line" | "pie" | "tree" | "stack");
    if kind == "table" && widget.fields.is_empty() {
        return Err(Ok(BoardReject::Fields));
    }
    if needs_group && (widget.group_field.is_empty() || !safe_ident(widget.group_field)) {
        return Err(Ok(BoardReject::Group));
    }
    if kind == "stack"
        && (widget.series_field.is_empty()
            || widget.series_field == widget.group_field
            || !safe_ident(widget.series_field))
    {
        return Err(Ok(BoardReject::Group));
    }
    if kind == "line" && !widget.group_is_date {
        return Err(Ok(BoardReject::LineDate));
    }
    if kind == "card" && !widget.trend_field.is_empty() && !widget.trend_is_date {
        return Err(Ok(BoardReject::LineDate));
    }
    if !widget.measure.is_empty() && (kind == "table" || !safe_ident(widget.measure)) {
        return Err(Ok(BoardReject::Fields));
    }
    let title = clean_label(widget.title, 60).ok_or(Ok(BoardReject::Title))?;
    let fields = if kind == "table" {
        widget.fields.join(",")
    } else {
        String::new()
    };
    if fields.len() > 800 {
        return Err(Ok(BoardReject::Fields));
    }
    Ok(PreparedWidget {
        title,
        fields,
        group: if needs_group {
            widget.group_field.to_owned()
        } else {
            String::new()
        },
        series: if kind == "stack" {
            widget.series_field.to_owned()
        } else {
            String::new()
        },
        measure: if kind == "table" {
            String::new()
        } else {
            widget.measure.to_owned()
        },
        trend: if kind == "card" {
            widget.trend_field.to_owned()
        } else {
            String::new()
        },
        span: if matches!(kind, "table" | "card") || widget.span != 6 {
            12
        } else {
            6
        },
    })
}

fn footprint(kind: &str) -> (i64, i64) {
    match kind {
        "card" => (6, 4),
        "table" => (30, 10),
        _ => (15, 8),
    }
}

fn initial_footprint(kind: &str, span: i64) -> (i64, i64) {
    match kind {
        "card" => (6, 4),
        "table" => (30, 10),
        _ if span == 6 => (15, 8),
        _ => (30, 8),
    }
}

fn min_footprint(kind: &str) -> (i64, i64) {
    match kind {
        "card" => (4, 3),
        "table" => (12, 6),
        _ => (8, 6),
    }
}

fn cell_of(widget: &Widget) -> Cell {
    Cell {
        x: widget.x,
        y: widget.y,
        w: widget.w,
        h: widget.h,
    }
}

impl Cell {
    fn covers(self, other: Cell) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

pub(crate) fn clamp_cell(kind: &str, x: i64, y: i64, w: i64, h: i64) -> Cell {
    let (min_w, min_h) = min_footprint(kind);
    let w = w.clamp(min_w, GRID_COLUMNS);
    let h = h.clamp(min_h, 24);
    let x = x.clamp(1, GRID_COLUMNS - w + 1);
    let y = y.clamp(1, 80);
    Cell { x, y, w, h }
}

fn first_slot(taken: &[Cell], w: i64, h: i64) -> Cell {
    let w = w.clamp(1, GRID_COLUMNS);
    let h = h.clamp(1, 24);
    for y in 1..120 {
        for x in 1..=(GRID_COLUMNS - w + 1) {
            let cell = Cell { x, y, w, h };
            if taken.iter().all(|other| !other.covers(cell)) {
                return cell;
            }
        }
    }
    Cell { x: 1, y: 120, w, h }
}

async fn settle_layout(db: &DatabaseConnection, widgets: &mut [Widget]) -> Result<(), FexuError> {
    if widgets.iter().all(|widget| widget.w > 0) {
        return Ok(());
    }
    let mut taken: Vec<Cell> = widgets
        .iter()
        .filter(|widget| widget.w > 0)
        .map(cell_of)
        .collect();
    let mut pending: Vec<usize> = widgets
        .iter()
        .enumerate()
        .filter(|(_, widget)| widget.w <= 0)
        .map(|(index, _)| index)
        .collect();
    pending.sort_by_key(|index| usize::from(widgets[*index].kind != "card"));
    for index in pending {
        let (w, h) = initial_footprint(&widgets[index].kind, widgets[index].span);
        let cell = first_slot(&taken, w, h);
        write_cell(db, widgets[index].id, cell).await?;
        widgets[index].x = cell.x;
        widgets[index].y = cell.y;
        widgets[index].w = cell.w;
        widgets[index].h = cell.h;
        taken.push(cell);
    }
    Ok(())
}

async fn write_cell(db: &DatabaseConnection, id: Uuid, cell: Cell) -> Result<(), FexuError> {
    let backend = db.get_database_backend();
    let mut query = Query::update();
    query
        .table(Alias::new(WIDGETS))
        .values([
            (Alias::new("grid_x"), DbValue::from(cell.x).into()),
            (Alias::new("grid_y"), DbValue::from(cell.y).into()),
            (Alias::new("grid_w"), DbValue::from(cell.w).into()),
            (Alias::new("grid_h"), DbValue::from(cell.h).into()),
        ])
        .and_where(Expr::col(Alias::new("id")).eq(id.to_string()));
    execute(db, backend, &query).await
}

async fn set_position(
    db: &DatabaseConnection,
    table: &str,
    id: &str,
    position: i64,
) -> Result<(), FexuError> {
    let backend = db.get_database_backend();
    let mut query = Query::update();
    query
        .table(Alias::new(table))
        .values([(Alias::new("position"), DbValue::from(position).into())])
        .and_where(Expr::col(Alias::new("id")).eq(id));
    execute(db, backend, &query).await
}

fn apply_filters(
    query: &mut fexu_db::driver::sea_query::SelectStatement,
    columns: &[ColumnSpec],
    filters: &[(&str, DbValue)],
) -> Result<(), FexuError> {
    for (column, value) in filters {
        if !safe_ident(column) || !columns.iter().any(|item| item.name == *column) {
            return Err(FexuError::Validation("filter is not available".into()));
        }
        query.and_where(Expr::col(Alias::new(*column)).eq(value.clone()));
    }
    Ok(())
}

fn amount_of(row: &fexu_db::driver::QueryResult) -> Result<i64, FexuError> {
    row.try_get::<i64>("", "amount")
        .or_else(|_| row.try_get::<i32>("", "amount").map(i64::from))
        .or_else(|_| {
            row.try_get::<f64>("", "amount")
                .map(|value| value.round() as i64)
        })
        .map_err(FexuError::database)
}

pub(crate) async fn grouped(
    source: AggregateQuery<'_>,
    field: &str,
    measure: &str,
    newest_first: bool,
) -> Result<(Vec<ChartPoint>, bool), FexuError> {
    let AggregateQuery {
        db,
        table,
        columns,
        scope,
        filters,
    } = source;
    if !columns.iter().any(|column| column.name == field)
        || !safe_ident(field)
        || !safe_ident(table)
    {
        return Err(FexuError::Validation("column is not available".into()));
    }
    let backend = db.get_database_backend();
    let mut query = Query::select();
    query.column(Alias::new(field));
    if measure.is_empty() {
        query.expr_as(Expr::col(Alias::new("id")).count(), Alias::new("amount"));
    } else {
        if !safe_ident(measure) || !columns.iter().any(|column| column.name == measure) {
            return Err(FexuError::Validation("measure is not available".into()));
        }
        query.expr_as(Expr::col(Alias::new(measure)).sum(), Alias::new("amount"));
    }
    query
        .from(Alias::new(table))
        .group_by_col(Alias::new(field))
        .limit(CHART_LIMIT + 1);
    for (column, value) in filters {
        if !safe_ident(column) || !columns.iter().any(|item| item.name == *column) {
            return Err(FexuError::Validation("filter is not available".into()));
        }
        query.and_where(Expr::col(Alias::new(*column)).eq(value.clone()));
    }
    if newest_first {
        query.order_by(Alias::new(field), Order::Desc);
    } else {
        query.order_by(Alias::new("amount"), Order::Desc);
    }
    apply_scope(&mut query, columns, scope)?;
    let rows = fetch(db, backend, &query).await?;
    let truncated = rows.len() > CHART_LIMIT as usize;
    let mut points = Vec::new();
    for row in rows.into_iter().take(CHART_LIMIT as usize) {
        let amount = row
            .try_get::<i64>("", "amount")
            .or_else(|_| row.try_get::<i32>("", "amount").map(i64::from))
            .or_else(|_| {
                row.try_get::<f64>("", "amount")
                    .map(|value| value.round() as i64)
            })
            .map_err(FexuError::database)?;
        points.push(ChartPoint {
            label: cell_label(&row, field),
            amount,
        });
    }
    if newest_first {
        points.reverse();
    }
    Ok((points, truncated))
}

pub(crate) fn group_is_date(columns: &[ColumnSpec], field: &str) -> bool {
    columns
        .iter()
        .any(|column| column.name == field && column.kind == ColumnKind::Date)
}

fn apply_scope(
    query: &mut fexu_db::driver::sea_query::SelectStatement,
    columns: &[ColumnSpec],
    scope: &QueryScope,
) -> Result<(), FexuError> {
    match scope {
        QueryScope::All => Ok(()),
        QueryScope::None => {
            query.and_where(Expr::val(0).eq(1));
            Ok(())
        }
        QueryScope::Equals { column, value } => {
            if !columns.iter().any(|item| item.name == *column) || !safe_ident(column) {
                return Err(FexuError::Validation(
                    "scope column is not available".into(),
                ));
            }
            let column = Expr::col(Alias::new(*column));
            match value {
                ScopeValue::Text(text) => {
                    query.and_where(column.eq(text.clone()));
                }
                ScopeValue::Uuid(id) => {
                    query.and_where(column.eq(id.to_string()));
                }
                ScopeValue::BigInt(number) => {
                    query.and_where(column.eq(*number));
                }
                ScopeValue::Bool(flag) => {
                    query.and_where(column.eq(*flag));
                }
            }
            Ok(())
        }
    }
}

fn cell_label(row: &fexu_db::driver::QueryResult, field: &str) -> String {
    if let Ok(text) = row.try_get::<String>("", field) {
        if text.is_empty() {
            return "—".to_owned();
        }
        return clip(&text, 48);
    }
    if let Ok(id) = row.try_get::<Uuid>("", field) {
        return id.to_string();
    }
    if let Ok(number) = row.try_get::<i64>("", field) {
        return number.to_string();
    }
    if let Ok(flag) = row.try_get::<bool>("", field) {
        return if flag { "true" } else { "false" }.to_owned();
    }
    "—".to_owned()
}

fn read_board(row: &fexu_db::driver::QueryResult) -> Result<Board, FexuError> {
    let id = row
        .try_get::<String>("", "id")
        .map_err(FexuError::database)?;
    let name = row
        .try_get::<String>("", "name")
        .map_err(FexuError::database)?;
    let home = row
        .try_get::<i64>("", "home")
        .or_else(|_| row.try_get::<i32>("", "home").map(i64::from))
        .unwrap_or(0);
    Ok(Board {
        id: Uuid::parse_str(&id)
            .map_err(|_| FexuError::Validation("stored uuid is invalid".into()))?,
        name,
        home: home != 0,
    })
}

fn read_widget(row: &fexu_db::driver::QueryResult) -> Result<Widget, FexuError> {
    let id = row
        .try_get::<String>("", "id")
        .map_err(FexuError::database)?;
    let fields = row
        .try_get::<String>("", "fields")
        .map_err(FexuError::database)?;
    Ok(Widget {
        id: Uuid::parse_str(&id)
            .map_err(|_| FexuError::Validation("stored uuid is invalid".into()))?,
        kind: row
            .try_get::<String>("", "kind")
            .map_err(FexuError::database)?,
        model_key: row
            .try_get::<String>("", "model_key")
            .map_err(FexuError::database)?,
        fields: fields
            .split(',')
            .filter(|field| !field.is_empty())
            .map(str::to_owned)
            .collect(),
        group_field: row
            .try_get::<String>("", "group_field")
            .map_err(FexuError::database)?,
        series_field: row
            .try_get::<String>("", "series_field")
            .unwrap_or_default(),
        measure: row.try_get::<String>("", "measure").unwrap_or_default(),
        trend_field: row.try_get::<String>("", "trend_field").unwrap_or_default(),
        span: row
            .try_get::<i64>("", "span")
            .or_else(|_| row.try_get::<i32>("", "span").map(i64::from))
            .unwrap_or(12),
        title: row
            .try_get::<String>("", "title")
            .map_err(FexuError::database)?,
        x: grid_number(row, "grid_x"),
        y: grid_number(row, "grid_y"),
        w: grid_number(row, "grid_w"),
        h: grid_number(row, "grid_h"),
    })
}

fn grid_number(row: &fexu_db::driver::QueryResult, field: &str) -> i64 {
    row.try_get::<i64>("", field)
        .or_else(|_| row.try_get::<i32>("", field).map(i64::from))
        .unwrap_or(0)
}

async fn count_rows(
    db: &DatabaseConnection,
    table: &str,
    board: Option<Uuid>,
) -> Result<i64, FexuError> {
    let backend = db.get_database_backend();
    let mut query = Query::select();
    query
        .expr_as(Expr::col(Alias::new("id")).count(), Alias::new("amount"))
        .from(Alias::new(table));
    if let Some(board) = board {
        query.and_where(Expr::col(Alias::new("board_id")).eq(board.to_string()));
    }
    let rows = fetch(db, backend, &query).await?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| FexuError::Validation("count missing".into()))?;
    row.try_get::<i64>("", "amount")
        .or_else(|_| row.try_get::<i32>("", "amount").map(i64::from))
        .map_err(FexuError::database)
}

fn clean_label(value: &str, max: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max
        || trimmed.chars().any(|char| char.is_control())
    {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn safe_ident(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    value.len() <= 64 && chars.all(|char| char.is_ascii_alphanumeric() || char == '_')
}

fn clip(value: &str, max: usize) -> String {
    let mut output = String::new();
    for (index, char) in value.chars().enumerate() {
        if index >= max {
            output.push('…');
            break;
        }
        output.push(char);
    }
    output
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

async fn execute_sql(
    db: &DatabaseConnection,
    backend: DatabaseBackend,
    sql: &str,
) -> Result<(), FexuError> {
    db.execute(Statement::from_string(backend, sql.to_owned()))
        .await
        .map(|_| ())
        .map_err(FexuError::database)
}

async fn execute(
    db: &impl ConnectionTrait,
    backend: DatabaseBackend,
    query: &impl fexu_db::driver::sea_query::QueryStatementWriter,
) -> Result<(), FexuError> {
    let (sql, values) = build(backend, query)?;
    db.execute(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map(|_| ())
        .map_err(FexuError::database)
}

async fn fetch(
    db: &impl ConnectionTrait,
    backend: DatabaseBackend,
    query: &impl fexu_db::driver::sea_query::QueryStatementWriter,
) -> Result<Vec<fexu_db::driver::QueryResult>, FexuError> {
    let (sql, values) = build(backend, query)?;
    db.query_all(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map_err(FexuError::database)
}

fn build(
    backend: DatabaseBackend,
    query: &impl fexu_db::driver::sea_query::QueryStatementWriter,
) -> Result<(String, Vec<DbValue>), FexuError> {
    let (sql, values) = match backend {
        DatabaseBackend::Postgres => query.build(PostgresQueryBuilder),
        DatabaseBackend::Sqlite => query.build(SqliteQueryBuilder),
        DatabaseBackend::MySql => {
            return Err(FexuError::Validation(
                "MySQL wird nicht unterstützt.".into(),
            ))
        }
    };
    Ok((sql, values.0))
}

#[cfg(test)]
mod tests {
    use super::{create_board, delete_board, list_boards};

    #[tokio::test]
    async fn a_board_round_trips_by_its_name() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        let id = create_board(&db, " Aufträge ").await.unwrap();
        let listed = list_boards(&db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].name, "Aufträge");
        delete_board(&db, id).await.unwrap();
        assert!(list_boards(&db).await.unwrap().is_empty());
    }
}
