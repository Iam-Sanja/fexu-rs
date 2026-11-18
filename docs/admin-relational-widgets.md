# Foreign key selection

[Overview](admin.md) · Security: [admin-security.md](admin-security.md)

**Milestone:** v0.17. The Cargo version is `0.1.2`.

The form offers a `<select>` containing up to 100 readable target records. If the current value is outside that page but readable, it is prepended. A “Continue searching” link opens the selection page.

The selection page is GET:

- `/admin/models/{modell}/pick/{feld}`
- `/admin/models/{modell}/{id}/pick/{feld}`

The selection page requires staff access, write permission on the source model and read permission on the target. Search and each page call `lookup_page` on the display label with the target read scope. Page size is the target `admin_per_page`, capped at 100. The reported total is that scoped count. An unknown field, hidden or read-only field, or target without a display field returns 404. Missing read permission returns 403. A unique foreign key is a one-to-one field (`data-relation="one_to_one"`). A target registered in another app is used when its table matches. Two models on the same table leave the picker empty.

A result links back to the source form with the field set to its database UUID. Other form values are not retained unless a draft is used. Requests cannot provide an arbitrary `next` destination; JavaScript is not required.

The destination is the registered admin model whose table equals `foreign_table`. The model may belong to another installed app. Two registered models with the same table are ambiguous: the picker stays empty and saving the field is rejected. An unregistered destination is left to the database foreign key check.

List and detail labels are loaded with one `WHERE id IN (...)` query per target table, in chunks of 100. A permitted row shows its display text and links to `/admin/models/{target}/{id}`. A missing row and a row the user may not read use the same label (`Nicht sichtbar` / `Not visible`). A superuser sees `Nicht vorhanden` / `Missing` for both. The raw UUID is not the visible label. The shortened id stays in the title of the secondary identifier.

When saving, `ensure_relations` rejects a UUID that the user cannot read at the unique registered destination. The answer remains the form with the error in the field.

After a validation error, a readable selection remains in the form because the transmitted field values are rendered again.

Many-to-many checkboxes use the same offset lookup of 100 rows. Search and paging redisplay the form without writing. A selected id outside the current page stays in a hidden field, and only an id that was shown can be cleared. `examples/admin-browser` and `tests/browser/admin-relations.spec.js` exercise the signed-in flows: foreign key, one-to-one, many-to-many search and paging, a scoped shelf, a forged id, inline create/update/delete, and a confirmed bulk action. Those tests also cover a 390-pixel viewport, keyboard focus, and the form without JavaScript. A manual screen reader was not run, and contrast was not measured.
