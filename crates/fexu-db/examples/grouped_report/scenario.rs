//! The same application code runs on SQLite and PostgreSQL.
//! Schema text comes from the framework renderer. Reports, filters and the
//! one-to-one load go through the public ORM. This file contains no SQL.

use fexu_core::{
    ChoiceValue, ColumnKind, ColumnSpec, DatabaseKind, FexuModel, ModelDescriptor, QueryScope,
    ScopeValue,
};
use fexu_db::driver::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use fexu_db::{
    col, insert_record, load_one_to_one, render_schema, Aggregate, Bound, Direction, GroupKey,
    Having, HavingCompare, Measure, MeasureValue, ModelQuery, NamedFilter,
};
use serde::Deserialize;
use uuid::Uuid;

fn column(name: &'static str, kind: ColumnKind, nullable: bool, primary_key: bool) -> ColumnSpec {
    ColumnSpec {
        name,
        kind,
        nullable,
        primary_key,
        default_bool: None,
        unique: false,
        indexed: false,
        foreign_table: None,
        foreign_column: None,
        on_delete: None,
        precision: 0,
        scale: 0,
        choices: &[],
    }
}

fn kind_of(db: &DatabaseConnection) -> Result<DatabaseKind, String> {
    match db.get_database_backend() {
        DatabaseBackend::Postgres => Ok(DatabaseKind::Postgres),
        DatabaseBackend::Sqlite => Ok(DatabaseKind::Sqlite),
        DatabaseBackend::MySql => Err("MySQL is not supported".into()),
    }
}

async fn create_model<M: FexuModel>(db: &DatabaseConnection) -> Result<(), String> {
    let kind = kind_of(db)?;
    db.execute(Statement::from_string(
        db.get_database_backend(),
        M::schema_sql(kind),
    ))
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Sale {
    id: Uuid,
    region: String,
    units: i64,
    owner_id: Uuid,
    label: Option<String>,
}

const SALE: ModelDescriptor = ModelDescriptor {
    name: "Sale",
    table: "fexu_report_sales",
    api_path: "",
    allow_anonymous: false,
    permission: None,
    requires_object_policy: false,
};

impl FexuModel for Sale {
    fn descriptor() -> &'static ModelDescriptor {
        &SALE
    }
    fn columns() -> &'static [ColumnSpec] {
        use std::sync::OnceLock;
        static COLUMNS: OnceLock<Vec<ColumnSpec>> = OnceLock::new();
        COLUMNS.get_or_init(|| {
            vec![
                column("id", ColumnKind::Uuid, false, true),
                column("region", ColumnKind::Text, false, false),
                column("units", ColumnKind::BigInt, false, false),
                column("owner_id", ColumnKind::Uuid, false, false),
                column("label", ColumnKind::Text, true, false),
            ]
        })
    }
    fn schema_sql(kind: DatabaseKind) -> String {
        render_schema(kind, Self::descriptor().table, Self::columns())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Profile {
    id: Uuid,
    user_id: Uuid,
    name: String,
}

const PROFILE: ModelDescriptor = ModelDescriptor {
    name: "Profile",
    table: "fexu_report_profiles",
    api_path: "",
    allow_anonymous: false,
    permission: None,
    requires_object_policy: false,
};

impl FexuModel for Profile {
    fn descriptor() -> &'static ModelDescriptor {
        &PROFILE
    }
    fn columns() -> &'static [ColumnSpec] {
        use std::sync::OnceLock;
        static COLUMNS: OnceLock<Vec<ColumnSpec>> = OnceLock::new();
        COLUMNS.get_or_init(|| {
            let mut user = column("user_id", ColumnKind::Uuid, false, false);
            user.unique = true;
            user.foreign_table = Some("fexu_report_users");
            user.foreign_column = Some("id");
            vec![
                column("id", ColumnKind::Uuid, false, true),
                user,
                column("name", ColumnKind::Text, false, false),
            ]
        })
    }
    fn schema_sql(kind: DatabaseKind) -> String {
        render_schema(kind, Self::descriptor().table, Self::columns())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Note {
    id: Uuid,
    body: Option<String>,
}

const NOTE: ModelDescriptor = ModelDescriptor {
    name: "Note",
    table: "fexu_report_notes",
    api_path: "",
    allow_anonymous: false,
    permission: None,
    requires_object_policy: false,
};

impl FexuModel for Note {
    fn descriptor() -> &'static ModelDescriptor {
        &NOTE
    }
    fn columns() -> &'static [ColumnSpec] {
        use std::sync::OnceLock;
        static COLUMNS: OnceLock<Vec<ColumnSpec>> = OnceLock::new();
        COLUMNS.get_or_init(|| {
            vec![
                column("id", ColumnKind::Uuid, false, true),
                column("body", ColumnKind::Json, true, false),
            ]
        })
    }
    fn schema_sql(kind: DatabaseKind) -> String {
        render_schema(kind, Self::descriptor().table, Self::columns())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Ledger {
    id: Uuid,
    amount: String,
}

const LEDGER: ModelDescriptor = ModelDescriptor {
    name: "Ledger",
    table: "fexu_report_ledger",
    api_path: "",
    allow_anonymous: false,
    permission: None,
    requires_object_policy: false,
};

impl FexuModel for Ledger {
    fn descriptor() -> &'static ModelDescriptor {
        &LEDGER
    }
    fn columns() -> &'static [ColumnSpec] {
        use std::sync::OnceLock;
        static COLUMNS: OnceLock<Vec<ColumnSpec>> = OnceLock::new();
        COLUMNS.get_or_init(|| {
            let mut amount = column("amount", ColumnKind::Decimal, false, false);
            amount.precision = 12;
            amount.scale = 2;
            vec![column("id", ColumnKind::Uuid, false, true), amount]
        })
    }
    fn schema_sql(kind: DatabaseKind) -> String {
        render_schema(kind, Self::descriptor().table, Self::columns())
    }
}

pub async fn exercise(db: &DatabaseConnection) -> Result<(), String> {
    create_model::<Sale>(db).await?;
    let owner = Uuid::from_u128(7);
    let other = Uuid::from_u128(8);
    for (id, region, units, owner, label) in [
        (1_u128, "north", 2_i64, owner, Some("aa")),
        (2, "north", 4, owner, Some("bbbb")),
        (3, "south", 9, owner, None),
        (4, "north", 100, other, Some("hidden")),
    ] {
        insert_record(
            db,
            Sale::descriptor().table,
            Sale::columns(),
            vec![
                Value::Uuid(Some(Box::new(Uuid::from_u128(id)))),
                Value::String(Some(Box::new(region.into()))),
                Value::BigInt(Some(units)),
                Value::Uuid(Some(Box::new(owner))),
                match label {
                    Some(text) => Value::String(Some(Box::new(text.into()))),
                    None => Value::String(None),
                },
            ],
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    let kept = NamedFilter::new("short_label", col("label").character_length_eq(2))
        .map_err(|error| error.to_string())?;
    let rows = Sale::query()
        .scope(QueryScope::Equals {
            column: "owner_id",
            value: ScopeValue::Uuid(owner),
        })
        .filter_named(&kept)
        .aggregate()
        .group_by(&["region"])
        .measure(Measure::count("sales"))
        .measure(Measure::sum("units_total", "units"))
        .measure(Measure::average("units_avg", "units"))
        .measure(Measure::min("units_min", "units"))
        .measure(Measure::max("label_max", "label"))
        .having(Having::new("units_total", HavingCompare::Gt, 0))
        .order_by_measure("units_total", Direction::Desc)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if rows.len() != 1 {
        return Err(format!("expected one visible group, got {rows:?}"));
    }
    let row = &rows[0];
    if row.keys.get("region") != Some(&GroupKey::Text("north".into()))
        || row.measures.get("sales") != Some(&MeasureValue::Integer(1))
        || row.measures.get("units_total") != Some(&MeasureValue::Integer(2))
        || row.measures.get("units_avg") != Some(&MeasureValue::Average("2.000000".into()))
        || row.measures.get("units_min") != Some(&MeasureValue::Integer(2))
        || row.measures.get("label_max") != Some(&MeasureValue::Text("aa".into()))
    {
        return Err(format!("grouped report row did not match: {row:?}"));
    }
    let labels = Sale::query()
        .scope(QueryScope::Equals {
            column: "owner_id",
            value: ScopeValue::Uuid(owner),
        })
        .aggregate()
        .group_by(&["label"])
        .measure(Measure::count("sales"))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if !labels
        .iter()
        .any(|group| group.keys.get("label") == Some(&GroupKey::Null))
    {
        return Err(format!("null label was not grouped: {labels:?}"));
    }
    if let Some(message) = [
        Sale::query()
            .aggregate()
            .group_by(&["region;drop"])
            .measure(Measure::count("sales"))
            .all(db)
            .await
            .map(|_| "unsafe group column was accepted"),
        Sale::query()
            .aggregate()
            .group_by(&["region"])
            .measure(Measure::count("sales;drop"))
            .all(db)
            .await
            .map(|_| "unsafe measure alias was accepted"),
        Sale::query()
            .aggregate()
            .group_by(&["region"])
            .measure(Measure::sum("bad", "region"))
            .all(db)
            .await
            .map(|_| "text column was summed"),
        Ledger::query()
            .aggregate()
            .measure(Measure::sum("bad", "amount"))
            .all(db)
            .await
            .map(|_| "decimal column was summed"),
        Sale::query()
            .aggregate()
            .group_by(&["region"])
            .measure(Measure::average("units_avg", "units"))
            .having(Having::new("units_avg", HavingCompare::Gt, 0))
            .all(db)
            .await
            .map(|_| "average was used in having"),
        Sale::query()
            .aggregate()
            .group_by(&["region"])
            .measure(Measure::average("units_avg", "units"))
            .order_by_measure("units_avg", Direction::Asc)
            .all(db)
            .await
            .map(|_| "average was used for ordering"),
        Sale::query()
            .aggregate()
            .group_by(&["region"])
            .measure(Measure::count("sales"))
            .limit(101)
            .all(db)
            .await
            .map(|_| "report limit above 100 was accepted"),
    ]
    .into_iter()
    .flatten()
    .next()
    {
        return Err(message.into());
    }

    create_model::<Profile>(db).await?;
    let user = Uuid::from_u128(3);
    insert_record(
        db,
        Profile::descriptor().table,
        Profile::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(9)))),
            Value::Uuid(Some(Box::new(user))),
            Value::String(Some(Box::new("ada".into()))),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    let profiles = load_one_to_one::<Profile>(db, "user_id", &[user], &QueryScope::All)
        .await
        .map_err(|error| error.to_string())?;
    if profiles.len() != 1 || profiles[0].1.name != "ada" {
        return Err(format!("one-to-one load failed: {profiles:?}"));
    }
    let hidden = load_one_to_one::<Profile>(db, "user_id", &[user], &QueryScope::None)
        .await
        .map_err(|error| error.to_string())?;
    if !hidden.is_empty() {
        return Err("one-to-one ignored an empty scope".into());
    }
    if load_one_to_one::<Sale>(db, "owner_id", &[owner], &QueryScope::All)
        .await
        .is_ok()
    {
        return Err("non-unique column was accepted as one-to-one".into());
    }
    insert_record(
        db,
        Profile::descriptor().table,
        Profile::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(10)))),
            Value::Uuid(Some(Box::new(user))),
            Value::String(Some(Box::new("second".into()))),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    if load_one_to_one::<Profile>(db, "user_id", &[user], &QueryScope::All)
        .await
        .is_ok()
    {
        return Err("duplicate one-to-one rows were returned".into());
    }

    create_model::<Note>(db).await?;
    let document = fexu_core::canonical_json_text(" { \"b\": 1, \"a\": [true, null] } ")
        .map_err(|code| code.to_owned())?;
    let parsed =
        serde_json::from_str::<serde_json::Value>(&document).map_err(|error| error.to_string())?;
    let stored = insert_record(
        db,
        Note::descriptor().table,
        Note::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(11)))),
            Value::Json(Some(Box::new(parsed))),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    if stored.get("body").and_then(|value| value.as_str()) != Some(document.as_str()) {
        return Err(format!("json document was not canonical: {stored}"));
    }
    let json_null = insert_record(
        db,
        Note::descriptor().table,
        Note::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(12)))),
            Value::Json(Some(Box::new(serde_json::Value::Null))),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    if json_null.get("body").and_then(|value| value.as_str()) != Some("null") {
        return Err(format!("json null was stored as sql null: {json_null}"));
    }
    let sql_null = insert_record(
        db,
        Note::descriptor().table,
        Note::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(13)))),
            Value::Json(None),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    if !sql_null.get("body").is_some_and(serde_json::Value::is_null) {
        return Err(format!(
            "sql null was stored as a json document: {sql_null}"
        ));
    }
    match insert_record(
        db,
        Note::descriptor().table,
        Note::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(14)))),
            Value::String(Some(Box::new("not-json".into()))),
        ],
    )
    .await
    {
        Ok(_) => return Err("invalid json was stored".into()),
        Err(error) if error.to_string().contains("validation.json") => {}
        Err(error) => {
            return Err(format!(
                "invalid json was not rejected before PostgreSQL saw it: {error}"
            ))
        }
    }
    if Note::query()
        .filter(col("body").eq(Bound::Text("{}".into())))
        .all(db)
        .await
        .is_ok()
    {
        return Err("json column accepted a filter".into());
    }
    if Note::query()
        .filter(col("body").character_length_eq(2))
        .all(db)
        .await
        .is_ok()
    {
        return Err("json column accepted a character length".into());
    }
    if fexu_core::canonical_json_text("").is_ok() || fexu_core::canonical_json_text("{").is_ok() {
        return Err("invalid json text was accepted".into());
    }
    if Note::query()
        .aggregate()
        .group_by(&["body"])
        .measure(Measure::count("notes"))
        .all(db)
        .await
        .is_ok()
    {
        return Err("json column was grouped".into());
    }
    exercise_monthly_revenue(db).await?;
    exercise_integer_bounds_and_window(db).await?;
    Ok(())
}

const INVOICE_STATUS: &[ChoiceValue] = &[
    ChoiceValue {
        value: "open",
        label_de: "Offen",
        label_en: "Open",
    },
    ChoiceValue {
        value: "paid",
        label_de: "Bezahlt",
        label_en: "Paid",
    },
    ChoiceValue {
        value: "void",
        label_de: "Storniert",
        label_en: "Void",
    },
];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct Invoice {
    id: Uuid,
    sold_on: Option<String>,
    status: String,
    cents: i64,
    owner_id: Uuid,
}

const INVOICE: ModelDescriptor = ModelDescriptor {
    name: "Invoice",
    table: "fexu_report_invoices",
    api_path: "",
    allow_anonymous: false,
    permission: None,
    requires_object_policy: false,
};

impl FexuModel for Invoice {
    fn descriptor() -> &'static ModelDescriptor {
        &INVOICE
    }
    fn columns() -> &'static [ColumnSpec] {
        use std::sync::OnceLock;
        static COLUMNS: OnceLock<Vec<ColumnSpec>> = OnceLock::new();
        COLUMNS.get_or_init(|| {
            let mut status = column("status", ColumnKind::Choice, false, false);
            status.choices = INVOICE_STATUS;
            vec![
                column("id", ColumnKind::Uuid, false, true),
                column("sold_on", ColumnKind::Date, true, false),
                status,
                column("cents", ColumnKind::BigInt, false, false),
                column("owner_id", ColumnKind::Uuid, false, false),
            ]
        })
    }
    fn schema_sql(kind: DatabaseKind) -> String {
        render_schema(kind, Self::descriptor().table, Self::columns())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct InvoiceLine {
    id: Uuid,
    invoice_id: Uuid,
    owner_id: Uuid,
}

const INVOICE_LINE: ModelDescriptor = ModelDescriptor {
    name: "InvoiceLine",
    table: "fexu_report_invoice_lines",
    api_path: "",
    allow_anonymous: false,
    permission: None,
    requires_object_policy: false,
};

impl FexuModel for InvoiceLine {
    fn descriptor() -> &'static ModelDescriptor {
        &INVOICE_LINE
    }
    fn columns() -> &'static [ColumnSpec] {
        use std::sync::OnceLock;
        static COLUMNS: OnceLock<Vec<ColumnSpec>> = OnceLock::new();
        COLUMNS.get_or_init(|| {
            let mut invoice = column("invoice_id", ColumnKind::Uuid, false, false);
            invoice.foreign_table = Some("fexu_report_invoices");
            invoice.foreign_column = Some("id");
            vec![
                column("id", ColumnKind::Uuid, false, true),
                invoice,
                column("owner_id", ColumnKind::Uuid, false, false),
            ]
        })
    }
    fn schema_sql(kind: DatabaseKind) -> String {
        render_schema(kind, Self::descriptor().table, Self::columns())
    }
}

fn date_value(text: &str) -> Result<Value, String> {
    let date = fexu_core::parse_date(text).map_err(|code| code.to_owned())?;
    Ok(Value::ChronoDate(Some(Box::new(date))))
}

async fn insert_invoice(
    db: &DatabaseConnection,
    id: u128,
    sold_on: Option<&str>,
    status: &str,
    cents: i64,
    owner: Uuid,
) -> Result<(), String> {
    insert_record(
        db,
        Invoice::descriptor().table,
        Invoice::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(id)))),
            match sold_on {
                Some(text) => date_value(text)?,
                None => Value::ChronoDate(None),
            },
            Value::String(Some(Box::new(status.to_owned()))),
            Value::BigInt(Some(cents)),
            Value::Uuid(Some(Box::new(owner))),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn exercise_monthly_revenue(db: &DatabaseConnection) -> Result<(), String> {
    create_model::<Invoice>(db).await?;
    create_model::<InvoiceLine>(db).await?;
    let owner = Uuid::from_u128(7);
    let other = Uuid::from_u128(8);
    let rows = [
        (21_u128, Some("2026-01-15"), "paid", 1_000_i64, owner),
        (22, Some("2026-01-20"), "paid", 500, owner),
        (23, Some("2026-01-22"), "open", 100, owner),
        (24, Some("2026-02-02"), "open", 200, owner),
        (25, Some("2026-02-03"), "void", 9_000, owner),
        (26, Some("2026-01-15"), "paid", 8_000, other),
        (27, None, "paid", 50, owner),
    ];
    for (id, sold_on, status, cents, row_owner) in rows {
        insert_invoice(db, id, sold_on, status, cents, row_owner).await?;
    }
    let active = NamedFilter::new("open", col("status").eq("open"))
        .map_err(|error| error.to_string())?
        .or(NamedFilter::new("paid", col("status").eq("paid"))
            .map_err(|error| error.to_string())?)
        .and(
            NamedFilter::new("void", col("status").eq("void"))
                .map_err(|error| error.to_string())?
                .negated(),
        );
    let scope = QueryScope::Equals {
        column: "owner_id",
        value: ScopeValue::Uuid(owner),
    };
    let listed = Invoice::query()
        .scope(scope.clone())
        .filter_named(&active)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if listed.len() != 5 {
        return Err(format!("shared filter kept the wrong invoices: {listed:?}"));
    }
    let report = Invoice::query()
        .scope(scope.clone())
        .filter_named(&active)
        .aggregate()
        .group_by_month("sold_on")
        .group_by(&["status"])
        .measure(Measure::count("invoices"))
        .measure(Measure::sum("cents_total", "cents"))
        .measure(Measure::min("cents_min", "cents"))
        .measure(Measure::max("cents_max", "cents"))
        .order_by_key("sold_on", Direction::Asc)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    let counted: i64 = report
        .iter()
        .map(|row| match row.measures.get("invoices") {
            Some(MeasureValue::Integer(value)) => *value,
            _ => 0,
        })
        .sum();
    if counted != listed.len() as i64 {
        return Err(format!(
            "list and report filters diverged: listed {} report {report:?}",
            listed.len()
        ));
    }
    let expect = [
        (Some("2026-01"), "open", 1, 100, 100, 100),
        (Some("2026-01"), "paid", 2, 1_500, 500, 1_000),
        (Some("2026-02"), "open", 1, 200, 200, 200),
        (None, "paid", 1, 50, 50, 50),
    ];
    if report.len() != expect.len() {
        return Err(format!("monthly report rows: {report:?}"));
    }
    for (row, (month, status, count, total, low, high)) in report.iter().zip(expect) {
        let key = match month {
            Some(text) => GroupKey::Text(text.into()),
            None => GroupKey::Null,
        };
        if row.keys.get("sold_on") != Some(&key)
            || row.keys.get("status") != Some(&GroupKey::Text(status.into()))
            || row.measures.get("invoices") != Some(&MeasureValue::Integer(count))
            || row.measures.get("cents_total") != Some(&MeasureValue::Integer(total))
            || row.measures.get("cents_min") != Some(&MeasureValue::Integer(low))
            || row.measures.get("cents_max") != Some(&MeasureValue::Integer(high))
        {
            return Err(format!(
                "monthly group did not match {month:?}/{status}: {row:?}"
            ));
        }
    }
    let hidden = Invoice::query()
        .scope(QueryScope::None)
        .filter_named(&active)
        .aggregate()
        .group_by_month("sold_on")
        .measure(Measure::sum("cents_total", "cents"))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if !hidden.is_empty() {
        return Err(format!("empty scope leaked grouped revenue: {hidden:?}"));
    }
    let empty = Invoice::query()
        .scope(scope.clone())
        .filter(col("sold_on").eq("1999-01-01"))
        .aggregate()
        .group_by_month("sold_on")
        .group_by(&["status"])
        .measure(Measure::count("invoices"))
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if !empty.is_empty() {
        return Err(format!("empty month filter returned rows: {empty:?}"));
    }
    let parent = Uuid::from_u128(21);
    for (id, line_owner) in [(31_u128, owner), (32, owner), (33, other)] {
        insert_record(
            db,
            InvoiceLine::descriptor().table,
            InvoiceLine::columns(),
            vec![
                Value::Uuid(Some(Box::new(Uuid::from_u128(id)))),
                Value::Uuid(Some(Box::new(parent))),
                Value::Uuid(Some(Box::new(line_owner))),
            ],
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    let counted_lines = Invoice::query()
        .scope(scope.clone())
        .filter(col("id").eq(parent))
        .annotate(
            db,
            Aggregate::<InvoiceLine>::count("lines", "invoice_id").scope(scope.clone()),
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
    if counted_lines.len() != 1 || counted_lines[0].value != 2 {
        return Err(format!(
            "related count ignored the child scope: {counted_lines:?}"
        ));
    }
    let denied_lines = Invoice::query()
        .scope(scope)
        .filter(col("id").eq(parent))
        .annotate(
            db,
            Aggregate::<InvoiceLine>::count("lines", "invoice_id").scope(QueryScope::None),
            None,
        )
        .await
        .map_err(|error| error.to_string())?;
    if denied_lines.len() != 1 || denied_lines[0].value != 0 {
        return Err(format!(
            "related count leaked rows outside the child scope: {denied_lines:?}"
        ));
    }
    if Invoice::query()
        .aggregate()
        .group_by_month("status")
        .measure(Measure::count("invoices"))
        .all(db)
        .await
        .is_ok()
    {
        return Err("choice column was grouped by month".into());
    }
    if Invoice::query()
        .aggregate()
        .group_by_month("sold_on; drop")
        .measure(Measure::count("invoices"))
        .all(db)
        .await
        .is_ok()
    {
        return Err("unsafe month column was accepted".into());
    }
    if Invoice::query()
        .filter(col("status").eq("not-a-status"))
        .aggregate()
        .group_by(&["status"])
        .measure(Measure::count("invoices"))
        .all(db)
        .await
        .is_ok()
    {
        return Err("unknown choice was aggregated".into());
    }
    if Ledger::query()
        .aggregate()
        .measure(Measure::min("low", "amount"))
        .all(db)
        .await
        .is_ok()
    {
        return Err("decimal column was aggregated with min".into());
    }
    Ok(())
}

async fn insert_sale(
    db: &DatabaseConnection,
    id: u128,
    region: &str,
    units: i64,
    owner: Uuid,
    label: Option<&str>,
) -> Result<(), String> {
    insert_record(
        db,
        Sale::descriptor().table,
        Sale::columns(),
        vec![
            Value::Uuid(Some(Box::new(Uuid::from_u128(id)))),
            Value::String(Some(Box::new(region.to_owned()))),
            Value::BigInt(Some(units)),
            Value::Uuid(Some(Box::new(owner))),
            match label {
                Some(text) => Value::String(Some(Box::new(text.to_owned()))),
                None => Value::String(None),
            },
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn exercise_integer_bounds_and_window(db: &DatabaseConnection) -> Result<(), String> {
    let owner = Uuid::from_u128(7);
    insert_sale(db, 40, "edge", i64::MAX, owner, Some("edge")).await?;
    match Sale::query()
        .filter(col("region").eq("edge"))
        .aggregate()
        .group_by(&["region"])
        .measure(Measure::average("units_avg", "units"))
        .all(db)
        .await
    {
        Err(error) if error.to_string().contains("portable range") => {}
        Ok(rows) => return Err(format!("overflowing average was returned: {rows:?}")),
        Err(error) => return Err(format!("overflowing average was not bounded: {error}")),
    }
    for number in 0_u128..101 {
        let region = format!("r{number:03}");
        insert_sale(db, 100 + number, &region, 1, owner, Some("zz")).await?;
    }
    let window = Sale::query()
        .filter(col("region").starts_with("r"))
        .aggregate()
        .group_by(&["region"])
        .measure(Measure::count("sales"))
        .measure(Measure::sum("units_total", "units"))
        .order_by_key("region", Direction::Asc)
        .limit(100)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if window.len() != 100 {
        return Err(format!("report window was not bounded: {}", window.len()));
    }
    if window.first().and_then(|row| row.keys.get("region")) != Some(&GroupKey::Text("r000".into()))
        || window.last().and_then(|row| row.keys.get("region"))
            != Some(&GroupKey::Text("r099".into()))
    {
        return Err(format!(
            "report window order was not stable: {:?} .. {:?}",
            window.first(),
            window.last()
        ));
    }
    if window
        .iter()
        .any(|row| row.keys.get("region") == Some(&GroupKey::Text("r100".into())))
    {
        return Err("report window included the group past the limit".into());
    }
    let tied = Sale::query()
        .filter(col("region").starts_with("r"))
        .filter(col("region").is_in(["r010", "r011"]))
        .aggregate()
        .group_by(&["region"])
        .measure(Measure::sum("units_total", "units"))
        .order_by_measure("units_total", Direction::Asc)
        .all(db)
        .await
        .map_err(|error| error.to_string())?;
    if tied.len() != 2
        || tied[0].keys.get("region") != Some(&GroupKey::Text("r010".into()))
        || tied[1].keys.get("region") != Some(&GroupKey::Text("r011".into()))
    {
        return Err(format!(
            "equal measures were not ordered by the group key: {tied:?}"
        ));
    }
    Ok(())
}
