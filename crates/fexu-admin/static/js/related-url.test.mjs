import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

const source = readFileSync(
    join(dirname(fileURLToPath(import.meta.url)), "admin.js"),
    "utf8",
);
const start = source.indexOf("function relatedRecordPath");
const end = source.indexOf("\nfor (const root", start);
const relatedRecordPath = runInNewContext(
    `${source.slice(start, end)}\nrelatedRecordPath`,
);

const id = "11111111-1111-4111-8111-111111111111";

assert.equal(
    relatedRecordPath("products", id, "view"),
    `/admin/models/products/${id}`,
);
assert.equal(
    relatedRecordPath("products", "../javascript:alert(1)", "delete"),
    "/admin/models/products/..%2Fjavascript%3Aalert(1)/delete",
);
assert.equal(
    relatedRecordPath("a/b", "c?d=e#f", "change"),
    "/admin/models/a%2Fb/c%3Fd%3De%23f/edit",
);
assert.equal(relatedRecordPath("", id, "view"), "");
assert.equal(relatedRecordPath("products", id, "add"), "");
