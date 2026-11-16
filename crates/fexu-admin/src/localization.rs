//! Feste Kataloge für die HTML-Oberfläche. Locale-Werte werden nie zu Dateipfaden.

use serde_json::{Map, Value};

pub fn normalize(requested: &str, default_locale: &str, fallback_locale: &str) -> String {
    match requested {
        "de" | "en" => requested.to_owned(),
        _ if default_locale == "de" || default_locale == "en" => default_locale.to_owned(),
        _ if fallback_locale == "de" || fallback_locale == "en" => fallback_locale.to_owned(),
        _ => "en".to_owned(),
    }
}

pub fn catalog(locale: &str) -> Value {
    let mut map = Map::new();
    for (key, german, english) in STRINGS {
        map.insert(
            (*key).to_owned(),
            Value::String(if locale == "de" {
                (*german).to_owned()
            } else {
                (*english).to_owned()
            }),
        );
    }
    Value::Object(map)
}

const STRINGS: &[(&str, &str, &str)] = &[
    ("login", "Anmelden", "Sign in"),
    ("email", "E-Mail", "Email"),
    ("password", "Passwort", "Password"),
    (
        "invalid_credentials",
        "E-Mail oder Passwort ist ungültig.",
        "Invalid email or password.",
    ),
    (
        "rate_limited",
        "Zu viele Versuche. Bitte später erneut versuchen.",
        "Too many attempts. Try again later.",
    ),
    (
        "csrf",
        "Die Sicherheitsprüfung ist fehlgeschlagen.",
        "The security check failed.",
    ),
    ("dashboard", "Dashboards", "Dashboards"),
    ("dashboards", "Dashboards", "Dashboards"),
    ("general", "Allgemein", "General"),
    ("board_config", "Konfiguration", "Configuration"),
    ("superuser_role", "Admin", "Admin"),
    ("staff_role", "Mitarbeiter", "Staff"),
    ("welcome", "Willkommen", "Welcome"),
    ("models", "Modelle", "Models"),
    ("users", "Benutzer", "Users"),
    ("roles", "Rollen", "Roles"),
    ("permissions", "Berechtigungen", "Permissions"),
    ("directory", "Verzeichnis", "Directory"),
    ("logout", "Abmelden", "Sign out"),
    ("account", "Konto", "Account"),
    ("settings", "Einstellungen", "Settings"),
    ("set_password", "Neues Passwort setzen", "Set a new password"),
    ("account_menu", "Kontomenü", "Account menu"),
    ("account_record", "Eigener Benutzerdatensatz", "Your user record"),
    (
        "account_hint",
        "Sprache und Darstellung gelten für diese Verwaltung. Weitere persönliche Einstellungen kommen auf diese Seite.",
        "Language and appearance apply to this admin. More personal settings will live on this page.",
    ),
    ("search", "Suchen", "Search"),
    ("search_title", "Suche", "Search"),
    (
        "search_admin",
        "Seiten und Datensätze suchen",
        "Search pages and records",
    ),
    (
        "search_prompt",
        "Suchbegriff eingeben.",
        "Enter a search term.",
    ),
    ("search_pages", "Seiten", "Pages"),
    ("theme_toggle", "Darstellung wechseln", "Switch appearance"),
    ("filter", "Filtern", "Filter"),
    ("filter_panel", "Filter", "Filter"),
    ("filter_by", "Nach", "By"),
    ("save", "Speichern", "Save"),
    (
        "save_continue",
        "Speichern und weiter bearbeiten",
        "Save and continue editing",
    ),
    (
        "save_add",
        "Speichern und weiteren Eintrag erstellen",
        "Save and add another",
    ),
    ("pick_more", "Weiter suchen", "Search further"),
    (
        "lookup_truncated",
        "Die Auswahl zeigt die ersten 100 Treffer. Weitere Werte stehen in der Suche.",
        "The list shows the first 100 matches. Use search for the rest.",
    ),
    ("pick_heading", "Eintrag wählen", "Choose a record"),
    (
        "pick_empty",
        "Kein passender Eintrag.",
        "No matching record.",
    ),
    ("pick_back", "Zurück zum Formular", "Back to the form"),
    (
        "invalid_relation",
        "Wählen Sie einen sichtbaren Eintrag.",
        "Choose a visible record.",
    ),
    (
        "form_errors",
        "Bitte korrigieren Sie die markierten Felder.",
        "Correct the marked fields.",
    ),
    (
        "m2m_hint",
        "Mehrere vorhandene Einträge können ausgewählt werden. Eine leere Auswahl entfernt die sichtbaren Verbindungen.",
        "Several existing records can be selected. An empty selection removes the visible links.",
    ),
    (
        "m2m_empty",
        "Keine auswählbaren Einträge.",
        "No records are available to select.",
    ),
    (
        "m2m_truncated",
        "Die Liste zeigt die ersten 100 sichtbaren Einträge. Bereits ausgewählte sichtbare Einträge bleiben erhalten.",
        "The list shows the first 100 visible records. Already selected visible records stay listed.",
    ),
    (
        "m2m_blocked",
        "Die verbundenen Einträge können mit diesem Konto nicht gelesen werden.",
        "This account cannot read the related records.",
    ),
    (
        "m2m_invalid",
        "Die Auswahl enthält einen ungültigen oder nicht sichtbaren Eintrag.",
        "The selection contains an invalid or hidden record.",
    ),
    (
        "m2m_size",
        "Es können höchstens 1000 Verbindungen gespeichert werden.",
        "At most 1000 links can be saved.",
    ),
    ("cancel", "Abbrechen", "Cancel"),
    ("create", "Anlegen", "Create"),
    ("edit", "Bearbeiten", "Edit"),
    ("delete", "Löschen", "Delete"),
    ("back", "Zurück", "Back"),
    ("detail", "Details", "Details"),
    ("empty", "Keine Einträge.", "No records."),
    (
        "no_models",
        "Keine Modelle für dieses Konto.",
        "No models are available for this account.",
    ),
    (
        "forbidden",
        "Diese Aktion ist nicht erlaubt.",
        "This action is not allowed.",
    ),
    (
        "not_found",
        "Der Eintrag wurde nicht gefunden.",
        "The record was not found.",
    ),
    (
        "staff_required",
        "Die Verwaltung verlangt einen Staff-Zugang.",
        "Administration requires staff access.",
    ),
    (
        "confirm_delete",
        "Dieser Eintrag wird dauerhaft gelöscht.",
        "This record will be deleted permanently.",
    ),
    (
        "notice_created",
        "Der Eintrag wurde angelegt.",
        "The record was created.",
    ),
    (
        "notice_saved",
        "Die Änderungen wurden gespeichert.",
        "The changes were saved.",
    ),
    (
        "notice_deleted",
        "Der Eintrag wurde gelöscht.",
        "The record was deleted.",
    ),
    (
        "notice_updated",
        "Die Änderung wurde gespeichert.",
        "The change was saved.",
    ),
    ("yes", "Ja", "Yes"),
    ("no", "Nein", "No"),
    ("active", "Aktiv", "Active"),
    ("inactive", "Inaktiv", "Inactive"),
    ("staff", "Staff", "Staff"),
    ("no_staff", "Kein Staff", "Not staff"),
    ("superuser", "Superuser", "Superuser"),
    ("no_superuser", "Kein Superuser", "Not a superuser"),
    ("actions", "Aktionen", "Actions"),
    ("image", "Bild", "Image"),
    ("required", "Pflichtfeld", "Required"),
    ("optional", "Optional", "Optional"),
    ("readonly", "Nur lesen", "Read only"),
    ("none", "Keine", "None"),
    ("total", "Gesamt", "Total"),
    ("previous", "Zurück", "Previous"),
    ("next", "Weiter", "Next"),
    ("page", "Seite", "Page"),
    ("language", "Sprache", "Language"),
    ("theme", "Darstellung", "Theme"),
    ("light", "Hell", "Light"),
    ("dark", "Dunkel", "Dark"),
    ("system", "System", "System"),
    ("apply", "Übernehmen", "Apply"),
    ("assign", "Zuweisen", "Assign"),
    ("revoke", "Entziehen", "Revoke"),
    ("grant", "Gewähren", "Grant"),
    ("role", "Rolle", "Role"),
    ("add_role", "Rolle anlegen", "Create role"),
    ("permission_code", "Berechtigungscode", "Permission code"),
    ("deactivate", "Deaktivieren", "Deactivate"),
    ("activate", "Aktivieren", "Activate"),
    ("remove_staff", "Staff entziehen", "Remove staff"),
    ("make_staff", "Staff geben", "Grant staff"),
    ("blocked", "Gesperrt", "Blocked"),
    ("open", "Öffnen", "Open"),
    ("new_record", "Neuer Eintrag", "New record"),
    ("results", "Treffer", "Results"),
    ("skip", "Zum Inhalt", "Skip to content"),
    ("navigation", "Navigation", "Navigation"),
    ("breadcrumb", "Pfad", "Breadcrumb"),
    ("reset", "Zurücksetzen", "Reset"),
    ("error", "Fehler", "Error"),
    ("sort_asc", "aufsteigend", "ascending"),
    ("sort_desc", "absteigend", "descending"),
    (
        "hint_select",
        "Wählen Sie einen vorhandenen Eintrag.",
        "Choose an existing record.",
    ),
    (
        "hint_flags",
        "Staff öffnet die Verwaltung. Superuser ersetzt Staff nicht.",
        "Staff opens administration. Superuser does not replace staff.",
    ),
    (
        "hint_role",
        "Nur vorhandene Rollen. Eine neue Rolle entsteht auf der Rollenseite.",
        "Existing roles only. Create a new role on the roles page.",
    ),
    (
        "hint_role_name",
        "Bis zu 80 Zeichen. Leerzeichen sind erlaubt, zum Beispiel Nur lesen.",
        "Up to 80 characters. Spaces are allowed, for example Read only.",
    ),
    (
        "hint_permission",
        "Je Tabelle legen Sie fest, ob die Rolle anlegen, lesen, ändern oder löschen darf.",
        "For each table, choose whether the role may create, read, update, or delete.",
    ),
    ("name", "Name", "Name"),
    (
        "hint_name",
        "Ein Name. Keine getrennten Felder für Vorname und Nachname.",
        "One name. No separate first and last name.",
    ),
    (
        "profile_heading",
        "Eigener Datensatz",
        "Your record",
    ),
    (
        "password_current",
        "Bisheriges Passwort",
        "Current password",
    ),
    (
        "password_signout",
        "Ein neues Passwort meldet Sie ab.",
        "A new password signs you out.",
    ),
    (
        "profile_invalid",
        "Name oder E-Mail ist ungültig.",
        "The name or email is invalid.",
    ),
    (
        "password_current_invalid",
        "Das bisherige Passwort stimmt nicht.",
        "The current password is wrong.",
    ),
    ("appearance", "Darstellung", "Appearance"),
    ("project", "Projekt", "Project"),
    (
        "project_hint",
        "Titel, Kopfzeile, Farbe und Symbole sieht der Browser. Währung und Sprachen gelten für die ganze Verwaltung.",
        "The browser shows the title, header, color, and icons. Currency and languages apply to the whole administration.",
    ),
    ("site_title_label", "Seitentitel", "Site title"),
    (
        "hint_site_title",
        "Steht im Titel des Browser-Tabs.",
        "Shown in the browser tab title.",
    ),
    ("site_header_label", "Kopfzeile", "Header"),
    (
        "hint_site_header",
        "Steht oben in der Seitenleiste.",
        "Shown at the top of the sidebar.",
    ),
    ("app_name_label", "Anwendung", "Application"),
    (
        "hint_app_name",
        "Name der Anwendung für den Browser.",
        "Application name for the browser.",
    ),
    ("primary_color_label", "Markenfarbe", "Brand color"),
    ("favicon_label", "Favicon", "Favicon"),
    (
        "apple_touch_label",
        "Apple-Touch-Symbol",
        "Apple touch icon",
    ),
    (
        "hint_icon",
        "PNG, JPEG, GIF, WebP oder ICO. Ohne neue Datei bleibt das bisherige Symbol.",
        "PNG, JPEG, GIF, WebP, or ICO. Without a new file the current icon stays.",
    ),
    ("icon_clear", "Entfernen", "Remove"),
    (
        "hint_image",
        "PNG, JPEG, GIF, WebP oder ICO. Ohne neue Datei bleibt das bisherige Bild.",
        "PNG, JPEG, GIF, WebP, or ICO. Without a new file the current image stays.",
    ),
    ("currency_label", "Währung", "Currency"),
    (
        "hint_currency",
        "Drei Buchstaben, zum Beispiel EUR.",
        "Three letters, for example EUR.",
    ),
    ("project_locale_label", "Standardsprache", "Default language"),
    ("project_fallback_label", "Ausweichsprache", "Fallback language"),
    (
        "project_invalid",
        "Die Projektangaben sind ungültig.",
        "The project details are invalid.",
    ),
    (
        "roles_grant",
        "Rollenvergabe",
        "Role assignment",
    ),
    (
        "hint_role_grant",
        "Rollen vergeben und ändern darf nur, wer diese Berechtigung hat. Ein Superuser darf es immer.",
        "Only someone with this permission can assign and change roles. A superuser always can.",
    ),
    ("action_create", "Anlegen", "Create"),
    ("action_read", "Lesen", "Read"),
    ("action_update", "Ändern", "Update"),
    ("action_delete", "Löschen", "Delete"),
    ("matrix_table", "Tabelle", "Table"),
    (
        "confirm_check",
        "Ich bestätige diese Änderung.",
        "I confirm this change.",
    ),
    ("assign_heading", "Rolle zuweisen", "Assign role"),
    (
        "no_roles",
        "Es gibt noch keine Rolle.",
        "There is no role yet.",
    ),
    ("existing_roles", "Vorhandene Rollen", "Existing roles"),
    (
        "unknown_role",
        "Diese Rolle gibt es nicht. Legen Sie sie zuerst an.",
        "That role does not exist. Create it first.",
    ),
    ("choose", "Bitte wählen", "Choose"),
    ("add_user", "Benutzer anlegen", "Add user"),
    (
        "password_confirm",
        "Passwort wiederholen",
        "Confirm password",
    ),
    (
        "password_rules",
        "12 bis 128 Zeichen. Das Passwort wird nicht angezeigt.",
        "12 to 128 characters. The password is not shown.",
    ),
    ("password_heading", "Passwort setzen", "Set password"),
    (
        "password_mismatch",
        "Die Passwörter stimmen nicht überein.",
        "The passwords do not match.",
    ),
    (
        "confirm_required",
        "Diese Aktion braucht eine Bestätigung.",
        "This action requires confirmation.",
    ),
    (
        "confirm_invalid",
        "Die Bestätigung ist ungültig.",
        "The confirmation is invalid.",
    ),
    ("missing_relation", "Nicht vorhanden", "Missing"),
    ("hidden_relation", "Nicht sichtbar", "Not visible"),
    ("all", "Alle", "All"),
    ("view", "Anzeigen", "View"),
    ("manage_roles", "Rollen", "Roles"),
    (
        "email_taken",
        "Diese E-Mail ist bereits vergeben.",
        "That email is already registered.",
    ),
    (
        "password_invalid",
        "Das Passwort muss 12 bis 128 Zeichen haben.",
        "The password must contain 12 to 128 characters.",
    ),
    ("no_permissions", "Keine Berechtigungen.", "No permissions."),
    (
        "user_saved",
        "Der Benutzer wurde gespeichert.",
        "The user was saved.",
    ),
    ("confirm", "Bestätigen", "Confirm"),
    ("history", "Historie", "History"),
    ("history_all", "Vollständige Historie", "Full history"),
    (
        "history_partial",
        "Nur Änderungen über die Administration. Direkte Repository-Schreibvorgänge und die Auth-CLI fehlen.",
        "Only changes made through the admin. Direct repository writes and the auth CLI are absent.",
    ),
    (
        "history_limit",
        "Die neuesten 8 Ereignisse.",
        "The latest 8 events.",
    ),
    (
        "history_more",
        "Weitere Ereignisse stehen in der vollständigen Historie.",
        "More events are in the full history.",
    ),
    (
        "history_none",
        "Für diesen Datensatz ist noch kein Eintrag gespeichert.",
        "No entry is stored for this record yet.",
    ),
    ("audit_create", "Angelegt", "Created"),
    ("audit_update", "Geändert", "Updated"),
    ("audit_delete", "Gelöscht", "Deleted"),
    ("audit_bulk_update", "Sammeländerung", "Bulk update"),
    ("audit_bulk_delete", "Sammellöschung", "Bulk delete"),
    ("audit_user_create", "Benutzer angelegt", "User created"),
    ("audit_user_update", "Benutzer geändert", "User updated"),
    (
        "audit_password_reset",
        "Passwort zurückgesetzt",
        "Password reset",
    ),
    ("audit_role_assign", "Rolle zugewiesen", "Role assigned"),
    ("audit_role_revoke", "Rolle entzogen", "Role revoked"),
    ("audit_permission_grant", "Recht vergeben", "Permission granted"),
    (
        "audit_permission_revoke",
        "Recht entzogen",
        "Permission revoked",
    ),
    (
        "dashboard_hint",
        "Die Listen stehen in der Seitennavigation.",
        "The lists are in the side navigation.",
    ),
    ("dashboard_activity", "Letzte Vorgänge", "Recent activity"),
    (
        "dashboard_activity_hint",
        "Diese Vorgänge stammen aus der Verwaltungshistorie. Ein Konto sieht die Vorgänge, die es sehen darf.",
        "These events come from the admin history. An account sees the events it is allowed to see.",
    ),
    (
        "dashboard_activity_error",
        "Die Vorgänge konnten nicht gelesen werden.",
        "The activity could not be read.",
    ),
    (
        "dashboard_empty_activity",
        "Noch keine Vorgänge.",
        "No activity yet.",
    ),
    ("dashboard_widgets", "Kennzahlen", "Figures"),
    ("board_new", "Neues Menü", "New menu"),
    ("board_name", "Name", "Name"),
    ("board_create", "Menü anlegen", "Create menu"),
    ("board_edit", "Menü bearbeiten", "Edit menu"),
    ("board_open", "Öffnen", "Open"),
    ("board_delete", "Menü löschen", "Delete menu"),
    ("board_add", "Block hinzufügen", "Add block"),
    ("board_continue", "Felder wählen", "Choose fields"),
    ("board_kind", "Darstellung", "Display"),
    ("board_table", "Tabelle", "Table"),
    ("board_bar", "Balken", "Bar"),
    ("board_line", "Linie", "Line"),
    ("board_pie", "Donut", "Donut"),
    ("board_stack", "Stapel", "Stacked"),
    ("board_tree", "Treemap", "Treemap"),
    ("board_card", "Karte", "Card"),
    ("board_home", "Beim Start öffnen", "Open at start"),
    ("board_start", "Startansicht", "Start view"),
    ("board_apply", "Übernehmen", "Apply"),
    ("board_preview", "Vorschau", "Preview"),
    (
        "board_preview_empty",
        "Die Vorschau zeigt den Block, sobald die Darstellung steht.",
        "The preview shows the block once the display is set.",
    ),
    ("board_home_on", "Dieses Menü öffnet beim Start.", "This menu opens at start."),
    ("board_series", "Serie", "Series"),
    ("board_trend", "Verlauf", "Trend"),
    (
        "board_layout_hint",
        "Auf dem Raster liegt die Ansicht. Verschieben setzt den Block auf eine freie Stelle, Größe ändert die Zellen. Beides gilt sofort.",
        "The grid is the view. Move sets the block on a free spot, Size changes the cells. Both apply at once.",
    ),
    ("board_drag", "Verschieben", "Move"),
    ("board_resize", "Größe", "Size"),
    ("board_place", "Platz", "Place"),
    ("board_column", "Spalte", "Column"),
    ("board_row", "Zeile", "Row"),
    ("board_width", "Breite", "Width"),
    ("board_height", "Höhe", "Height"),
    (
        "board_place_busy",
        "Dieser Platz ist schon belegt.",
        "That spot is already taken.",
    ),
    ("board_span", "Breite", "Width"),
    ("board_span_half", "Halbe Zeile", "Half row"),
    ("board_span_full", "Ganze Zeile", "Full row"),
    ("board_move_up", "Nach oben", "Move up"),
    ("board_move_down", "Nach unten", "Move down"),
    ("board_widget_edit", "Bearbeiten", "Edit"),
    ("board_model", "Modell", "Model"),
    ("board_fields", "Felder", "Fields"),
    ("board_group", "Gruppierung", "Grouping"),
    ("board_group_none", "Keine", "None"),
    ("board_measure", "Summierter Wert", "Summed value"),
    ("board_title", "Titel", "Title"),
    ("board_save", "Block speichern", "Save block"),
    ("board_remove", "Block entfernen", "Remove block"),
    (
        "board_empty",
        "Dieses Menü hat noch keinen Block.",
        "This menu has no block yet.",
    ),
    ("board_none", "Noch kein Menü.", "No menu yet."),
    (
        "board_hint",
        "Eine Tabelle zeigt die gewählten Felder. Eine Grafik gruppiert nach einem Feld und zählt die Datensätze oder summiert einen Zahlenwert. Eine Linie braucht ein Datumsfeld.",
        "A table shows the chosen fields. A chart groups by one field and counts the records or sums a number. A line needs a date field.",
    ),
    (
        "board_chart_limit",
        "Die Grafik zeigt die 12 größten Gruppen.",
        "The chart shows the 12 largest groups.",
    ),
    (
        "board_line_limit",
        "Die Linie zeigt die 12 letzten Gruppen.",
        "The line shows the 12 latest groups.",
    ),
    (
        "board_locked",
        "Dieses Modell ist für dieses Konto gesperrt.",
        "This model is locked for this account.",
    ),
    (
        "board_missing",
        "Das Modell oder ein Feld ist nicht mehr vorhanden.",
        "The model or a field is no longer available.",
    ),
    (
        "board_read_error",
        "Die Menüs konnten nicht gelesen werden.",
        "The menus could not be read.",
    ),
    (
        "board_name_invalid",
        "Der Name braucht 1 bis 48 Zeichen.",
        "The name needs 1 to 48 characters.",
    ),
    (
        "board_title_invalid",
        "Der Titel braucht 1 bis 60 Zeichen.",
        "The title needs 1 to 60 characters.",
    ),
    (
        "board_limit",
        "Es sind höchstens 24 Menüs möglich.",
        "At most 24 menus are possible.",
    ),
    (
        "board_widget_limit",
        "Ein Menü hat höchstens 12 Blöcke.",
        "A menu has at most 12 blocks.",
    ),
    (
        "board_fields_required",
        "Eine Tabelle braucht mindestens ein Feld.",
        "A table needs at least one field.",
    ),
    (
        "board_group_required",
        "Eine Grafik braucht ein Gruppierungsfeld.",
        "A chart needs a grouping field.",
    ),
    (
        "board_line_date",
        "Eine Linie braucht ein Datumsfeld.",
        "A line needs a date field.",
    ),
    ("board_bad_kind", "Diese Darstellung gibt es nicht.", "This display does not exist."),
    (
        "board_bad_model",
        "Dieses Modell steht nicht zur Verfügung.",
        "This model is not available.",
    ),
    (
        "dashboard_widgets_empty",
        "Tabellen und Grafiken legt man unter Dashboards in der Konfiguration an. Dort wählt man Modell, Felder und Darstellung.",
        "Tables and charts are created under Dashboards in Configuration. There you choose the model, fields and display.",
    ),
    ("open_list", "Liste öffnen", "Open list"),
    (
        "empty_filtered",
        "Keine Treffer für diese Suche oder diese Filter.",
        "No records match this search or these filters.",
    ),
    ("bulk_page", "Gesamte Seite", "Entire page"),
    ("bulk_action", "Aktion", "Action"),
    ("bulk_run", "Ausführen", "Run"),
    ("select", "Auswählen", "Select"),
    ("bulk_confirm", "Auswahl bestätigen", "Confirm selection"),
    ("bulk_count", "Betroffene Datensätze", "Affected records"),
    (
        "bulk_empty",
        "Keine Datensätze ausgewählt.",
        "No records selected.",
    ),
    (
        "bulk_denied",
        "Die Auswahl enthält einen nicht erlaubten Datensatz. Es wurde nichts geändert.",
        "The selection contains a record that is not allowed. Nothing was changed.",
    ),
    (
        "bulk_failed",
        "Die Aktion wurde zurückgerollt. Es wurde nichts geändert.",
        "The action was rolled back. Nothing was changed.",
    ),
    ("inline_add", "Hinzufügen", "Add"),
    (
        "inline_empty",
        "Keine verknüpften Einträge.",
        "No related records.",
    ),
    ("inline_open", "Öffnen", "Open"),
    ("inline_another", "Weitere Zeile", "Add another row"),
    ("inline_remove", "Entfernen", "Remove"),
    (
        "inline_marked",
        "Wird beim Speichern entfernt.",
        "Removed when you save.",
    ),
    (
        "inline_conflict",
        "Ein anderer Vorgang hat den Datensatz geändert. Es wurde nichts gespeichert.",
        "Another change updated this record. Nothing was saved.",
    ),
    (
        "inline_failed",
        "Die Änderung wurde zurückgerollt. Es wurde nichts gespeichert.",
        "The change was rolled back. Nothing was saved.",
    ),
    (
        "inline_limit",
        "Die Anzahl der Zeilen ist ungültig.",
        "The number of rows is invalid.",
    ),
    (
        "inline_foreign",
        "Die Zeile gehört nicht zu diesem Datensatz.",
        "The row does not belong to this record.",
    ),
    ("audit_empty", "Keine Einträge.", "No entries."),
    ("when", "Zeitpunkt", "When"),
    ("actor", "Akteur", "Actor"),
    ("outcome", "Ergebnis", "Outcome"),
    ("details", "Details", "Details"),
    ("record", "Datensatz", "Record"),
    ("succeeded", "Erfolgreich", "Succeeded"),
    ("denied", "Verweigert", "Denied"),
    ("date_hierarchy", "Datum", "Date"),
    (
        "utc_hint",
        "UTC. Der Wert wird nicht in eine lokale Zeitzone umgerechnet.",
        "UTC. The value is not converted to a local time zone.",
    ),
    (
        "validation_date",
        "Das Datum ist ungültig.",
        "The date is invalid.",
    ),
    (
        "validation_time",
        "Die Uhrzeit ist ungültig.",
        "The time is invalid.",
    ),
    (
        "validation_datetime",
        "Der Zeitpunkt ist ungültig. Erwartet wird UTC.",
        "The timestamp is invalid. UTC is required.",
    ),
    ("month_1", "Januar", "January"),
    ("month_2", "Februar", "February"),
    ("month_3", "März", "March"),
    ("month_4", "April", "April"),
    ("month_5", "Mai", "May"),
    ("month_6", "Juni", "June"),
    ("month_7", "Juli", "July"),
    ("month_8", "August", "August"),
    ("month_9", "September", "September"),
    ("month_10", "Oktober", "October"),
    ("month_11", "November", "November"),
    ("month_12", "Dezember", "December"),
    ("applications", "Anwendungen", "Applications"),
    ("access", "Benutzer und Zugang", "Users and access"),
    ("system", "System", "System"),
    (
        "time_utc",
        "Zeitpunkte werden in UTC angezeigt. Gespeicherte Werte bleiben unverändert.",
        "Times are shown in UTC. Stored values stay unchanged.",
    ),
    (
        "time_unavailable",
        "Zeitpunkt nicht lesbar",
        "Time unavailable",
    ),
    ("menu", "Menü", "Menu"),
];

pub fn notice(locale: &str, code: &str) -> Option<String> {
    let catalog = catalog(locale);
    let key = match code {
        "created" => "notice_created",
        "saved" => "notice_saved",
        "deleted" => "notice_deleted",
        "updated" => "notice_updated",
        "changed" => "notice_updated",
        _ => return None,
    };
    catalog
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

pub fn bulk_notice(locale: &str, count: u32) -> String {
    if locale == "de" {
        format!("{count} Datensätze wurden geändert.")
    } else {
        format!("{count} records were changed.")
    }
}

pub fn validation_text(locale: &str, detail: &str) -> String {
    let german = locale == "de";
    if detail.contains("must not be blank") || detail.contains("validation.null") {
        return if german {
            "Pflichtfeld.".into()
        } else {
            "Required.".into()
        };
    }
    if detail.contains("must have at least") || detail.contains("validation.min_length") {
        return if german {
            "Der Wert ist zu kurz.".into()
        } else {
            "The value is too short.".into()
        };
    }
    if detail.contains("may have at most") || detail.contains("validation.max_length") {
        return if german {
            "Der Wert ist zu lang.".into()
        } else {
            "The value is too long.".into()
        };
    }
    if detail.contains("not writable") {
        return if german {
            "Dieses Feld kann nicht gesetzt werden.".into()
        } else {
            "This field cannot be set.".into()
        };
    }
    if detail.contains("validation.date") {
        return catalog(locale)["validation_date"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.time") {
        return catalog(locale)["validation_time"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.datetime") {
        return catalog(locale)["validation_datetime"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.min_value") || detail.contains("must be >=") {
        return fexu_core::message_text(locale, "validation.min_value").to_owned();
    }
    if detail.contains("validation.email") || detail.contains("must be an email") {
        return fexu_core::message_text(locale, "validation.email").to_owned();
    }
    if detail.contains("validation.slug") || detail.contains("lowercase slug") {
        return fexu_core::message_text(locale, "validation.slug").to_owned();
    }
    if detail.contains("validation.choice") {
        return fexu_core::message_text(locale, "validation.choice").to_owned();
    }
    if detail.contains("validation.postal") {
        return fexu_core::message_text(locale, "validation.postal").to_owned();
    }
    if detail.contains("validation.required") {
        return if german {
            "Pflichtfeld.".into()
        } else {
            "Required.".into()
        };
    }
    if detail.contains("validation.inline_max") {
        return catalog(locale)["inline_limit"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.inline_duplicate") {
        return if german {
            "Eine Zeile ist doppelt.".into()
        } else {
            "A row is listed twice.".into()
        };
    }
    if detail.contains("validation.inline_foreign") || detail.contains("validation.readonly") {
        return catalog(locale)["inline_foreign"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.inline_field") {
        return if german {
            "Eine Zeile enthält ein ungültiges Feld.".into()
        } else {
            "A row contains an invalid field.".into()
        };
    }
    if detail.contains("conflict.stale") {
        return catalog(locale)["inline_conflict"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("admin.relation.size") {
        return catalog(locale)["m2m_size"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("admin.store") {
        return if german {
            "Die Datei konnte nicht gespeichert werden.".into()
        } else {
            "The file could not be stored.".into()
        };
    }
    if detail.contains("admin.relation") {
        return catalog(locale)["m2m_invalid"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.inline_failed") {
        return catalog(locale)["inline_failed"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail == "Invalid email or password" {
        return catalog(locale)["invalid_credentials"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    detail.to_owned()
}
