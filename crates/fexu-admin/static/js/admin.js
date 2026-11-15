const invalid = document.querySelector("[aria-invalid='true']");
if (invalid instanceof HTMLElement) {
    invalid.focus();
}

function fingerprint(form) {
    const skip = new Set(["csrf_token", "save", "confirm"]);
    const parts = [];
    const data = new FormData(form);
    for (const [key, value] of data.entries()) {
        if (skip.has(key)) {
            continue;
        }
        parts.push(`${key}=${String(value)}`);
    }
    parts.sort();
    return parts.join("&");
}

function track(form) {
    const mode = form.getAttribute("data-track-dirty");
    if (!mode) {
        return;
    }
    const buttons = [...form.querySelectorAll(".js-save")];
    if (buttons.length === 0) {
        return;
    }
    const initial = fingerprint(form);
    const forced = mode === "record" && form.querySelector(".field-error, [role='alert']") !== null;
    const apply = () => {
        delete form.dataset.submitting;
        const dirty = forced || fingerprint(form) !== initial;
        for (const button of buttons) {
            button.disabled = !dirty;
        }
        form.dataset.dirty = dirty ? "1" : "0";
    };
    apply();
    window.addEventListener("pageshow", apply);
    form.addEventListener("input", apply);
    form.addEventListener("change", apply);
    // A successful native submit may navigate away. Do not warn that the form
    // has unsaved changes when the user explicitly chose to save them.
    form.addEventListener("submit", (event) => {
        if (!event.defaultPrevented && form.checkValidity()) {
            form.dataset.submitting = "1";
        }
    });
    form.addEventListener("invalid", () => {
        delete form.dataset.submitting;
    }, true);
}

for (const form of document.querySelectorAll("form[data-track-dirty]")) {
    track(form);
}

window.addEventListener("beforeunload", (event) => {
    if (document.querySelector("form[data-dirty='1']:not([data-submitting='1'])") === null) {
        return;
    }
    event.preventDefault();
    event.returnValue = "";
});

// Table row navigation is progressive enhancement. The first cell remains
// a normal keyboard-accessible link. Native controls must keep their behavior.
document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.ctrlKey || event.metaKey
        || event.shiftKey || event.altKey || window.getSelection()?.toString()
        || !(event.target instanceof Element)) {
        return;
    }
    if (event.target.closest("a, button, input, select, textarea, label, summary, details, [role='button'], [role='link'], [contenteditable]:not([contenteditable='false']), [tabindex]:not([tabindex='-1'])")) {
        return;
    }
    const row = event.target.closest("tr[data-record-href]");
    if (row instanceof HTMLTableRowElement && row.dataset.recordHref) {
        window.location.assign(row.dataset.recordHref);
    }
});

document.addEventListener("click", (event) => {
    const link = event.target instanceof Element ? event.target.closest("a[data-popup]") : null;
    if (!(link instanceof HTMLAnchorElement) || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
    }
    event.preventDefault();
    const address = new URL(link.href, window.location.origin);
    address.searchParams.set("popup", "1");
    const opened = window.open(address.toString(), "fexu-related", "popup=yes,width=820,height=720,resizable=yes,scrollbars=yes");
    if (opened) {
        opened.focus();
    } else {
        window.location.assign(address.toString());
    }
});

for (const form of document.querySelectorAll("form[data-autosubmit]")) {
    if (!(form instanceof HTMLFormElement)) {
        continue;
    }
    const fallback = form.querySelector("[data-autosubmit-fallback]");
    if (fallback instanceof HTMLElement) {
        fallback.hidden = true;
    }
    form.addEventListener("change", () => form.requestSubmit());
}

const previewForm = document.querySelector("form.studio-form[data-preview]");
const previewBody = document.querySelector("[data-preview-body]");
if (previewForm instanceof HTMLFormElement && previewBody instanceof HTMLElement) {
    let timer = 0;
    const paint = () => {
        const data = new URLSearchParams(new FormData(previewForm));
        fetch(previewForm.dataset.preview, {
            method: "POST",
            body: data,
            headers: { "X-Preview": "fragment" },
        }).then((response) => (response.ok ? response.text() : "")).then((html) => {
            if (html) {
                previewBody.innerHTML = html;
            }
        }).catch(() => {});
    };
    const schedule = () => {
        window.clearTimeout(timer);
        timer = window.setTimeout(paint, 250);
    };
    previewForm.addEventListener("input", schedule);
    previewForm.addEventListener("change", schedule);
    previewForm.addEventListener("click", (event) => {
        const button = event.target instanceof Element ? event.target.closest("button[formaction]") : null;
        if (!(button instanceof HTMLButtonElement)) {
            return;
        }
        event.preventDefault();
        paint();
    });
}

for (const canvas of document.querySelectorAll(".board-canvas")) {
    const fit = () => {
        canvas.style.setProperty("--board-cell", `${canvas.clientWidth / 30}px`);
    };
    fit();
    new ResizeObserver(fit).observe(canvas);
}

const arrange = document.querySelector("[data-arrange]");
if (arrange instanceof HTMLElement) {
    const columns = 30;
    const csrf = arrange.dataset.csrf || "";
    const board = arrange.dataset.board || "";
    const limits = { card: [4, 3], table: [12, 6] };
    const minimum = (kind) => limits[kind] || [8, 6];
    const read = (item) => ({
        x: Number(item.dataset.x),
        y: Number(item.dataset.y),
        w: Number(item.dataset.w),
        h: Number(item.dataset.h),
    });
    const paint = (item, cell) => {
        item.style.gridColumn = `${cell.x} / span ${cell.w}`;
        item.style.gridRow = `${cell.y} / span ${cell.h}`;
    };
    const hits = (item, cell) => {
        for (const other of arrange.querySelectorAll("[data-widget]")) {
            if (other === item) {
                continue;
            }
            const box = read(other);
            if (cell.x < box.x + box.w && box.x < cell.x + cell.w && cell.y < box.y + box.h && box.y < cell.y + cell.h) {
                return true;
            }
        }
        return false;
    };
    const remember = (item, cell) => {
        const form = document.querySelector("form.studio-form");
        const id = form instanceof HTMLFormElement ? form.querySelector("[name=widget]") : null;
        if (!(form instanceof HTMLFormElement) || !(id instanceof HTMLInputElement) || id.value !== item.dataset.widget) {
            return;
        }
        form.elements.grid_x.value = String(cell.x);
        form.elements.grid_y.value = String(cell.y);
        form.elements.grid_w.value = String(cell.w);
        form.elements.grid_h.value = String(cell.h);
    };
    const save = (item, cell, origin) => {
        const data = new URLSearchParams();
        data.set("csrf_token", csrf);
        data.set("grid_x", String(cell.x));
        data.set("grid_y", String(cell.y));
        data.set("grid_w", String(cell.w));
        data.set("grid_h", String(cell.h));
        fetch(`/admin/boards/${board}/widgets/${item.dataset.widget}/place`, {
            method: "POST",
            body: data,
            headers: { "X-Place": "fragment" },
        }).then((response) => {
            if (!response.ok) {
                paint(item, origin);
                remember(item, origin);
                return;
            }
            item.dataset.x = String(cell.x);
            item.dataset.y = String(cell.y);
            item.dataset.w = String(cell.w);
            item.dataset.h = String(cell.h);
            remember(item, cell);
        }).catch(() => {
            paint(item, origin);
            remember(item, origin);
        });
    };
    arrange.addEventListener("pointerdown", (event) => {
        const handle = event.target instanceof Element ? event.target.closest("[data-drag], [data-resize]") : null;
        if (!(handle instanceof HTMLElement)) {
            return;
        }
        const item = handle.closest("[data-widget]");
        if (!(item instanceof HTMLElement)) {
            return;
        }
        event.preventDefault();
        handle.setPointerCapture(event.pointerId);
        const origin = read(item);
        const pitch = () => arrange.clientWidth / columns;
        const startX = event.clientX;
        const startY = event.clientY;
        const resizing = handle.hasAttribute("data-resize");
        let next = origin;
        const move = (ev) => {
            const step = pitch();
            const dx = Math.round((ev.clientX - startX) / step);
            const dy = Math.round((ev.clientY - startY) / step);
            const [minW, minH] = minimum(item.dataset.kind || "");
            if (resizing) {
                next = {
                    x: origin.x,
                    y: origin.y,
                    w: Math.min(columns - origin.x + 1, Math.max(minW, origin.w + dx)),
                    h: Math.max(minH, Math.min(24, origin.h + dy)),
                };
            } else {
                next = {
                    x: Math.min(columns - origin.w + 1, Math.max(1, origin.x + dx)),
                    y: Math.max(1, Math.min(80, origin.y + dy)),
                    w: origin.w,
                    h: origin.h,
                };
            }
            item.classList.toggle("is-blocked", hits(item, next));
            paint(item, next);
        };
        const up = (ev) => {
            handle.removeEventListener("pointermove", move);
            handle.removeEventListener("pointerup", up);
            handle.removeEventListener("pointercancel", up);
            move(ev);
            item.classList.remove("is-blocked");
            if (hits(item, next)) {
                paint(item, origin);
                return;
            }
            if (next.x === origin.x && next.y === origin.y && next.w === origin.w && next.h === origin.h) {
                return;
            }
            save(item, next, origin);
        };
        handle.addEventListener("pointermove", move);
        handle.addEventListener("pointerup", up);
        handle.addEventListener("pointercancel", up);
    });
}

for (const input of document.querySelectorAll("input[data-keep-query]")) {
    input.value = window.location.pathname + window.location.search;
}

for (const root of document.querySelectorAll("[data-related-root]")) {
    const select = root.querySelector("select");
    const key = root.getAttribute("data-related-key");
    if (!(select instanceof HTMLSelectElement) || !key) {
        continue;
    }
    const sync = () => {
        const id = select.value;
        for (const link of root.querySelectorAll("[data-related]")) {
            const kind = link.getAttribute("data-related");
            if (kind === "add") {
                continue;
            }
            if (!id) {
                link.hidden = true;
                continue;
            }
            if (kind === "view") {
                link.href = `/admin/models/${key}/${id}`;
            } else if (kind === "change") {
                link.href = `/admin/models/${key}/${id}/edit`;
            } else if (kind === "delete") {
                link.href = `/admin/models/${key}/${id}/delete`;
            }
            link.hidden = false;
        }
    };
    select.addEventListener("change", sync);
}

function placeMenu(menu) {
    const panel = menu.querySelector(".menu-panel");
    const summary = menu.querySelector("summary");
    if (!(panel instanceof HTMLElement) || !(summary instanceof HTMLElement)) {
        return;
    }
    const box = summary.getBoundingClientRect();
    const width = panel.offsetWidth;
    const height = panel.offsetHeight;
    let top = box.bottom + 4;
    if (top + height > window.innerHeight - 8) {
        top = Math.max(8, box.top - height - 4);
    }
    let left = box.right - width;
    if (left < 8) {
        left = 8;
    }
    panel.style.position = "fixed";
    panel.style.top = `${top}px`;
    panel.style.left = `${left}px`;
    panel.style.right = "auto";
    panel.style.zIndex = "40";
}

function clearMenu(menu) {
    const panel = menu.querySelector(".menu-panel");
    if (!(panel instanceof HTMLElement)) {
        return;
    }
    panel.style.position = "";
    panel.style.top = "";
    panel.style.left = "";
    panel.style.right = "";
}

document.addEventListener("toggle", (event) => {
    const menu = event.target;
    if (!(menu instanceof HTMLDetailsElement) || !menu.classList.contains("menu")) {
        return;
    }
    if (!menu.open) {
        clearMenu(menu);
        return;
    }
    for (const other of document.querySelectorAll("details.menu[open]")) {
        if (other !== menu) {
            other.open = false;
        }
    }
    placeMenu(menu);
}, true);

function listValue(control) {
    if (control instanceof HTMLInputElement && control.type === "checkbox") {
        return control.checked ? "true" : "false";
    }
    return control.value;
}

document.addEventListener("input", markRow);
document.addEventListener("change", markRow);
function markRow(event) {
    const control = event.target;
    if (!(control instanceof HTMLElement) || !control.hasAttribute("data-list-edit")) {
        return;
    }
    const row = control.closest("tr");
    const button = row instanceof HTMLTableRowElement ? row.querySelector("[data-row-save]") : null;
    if (!(button instanceof HTMLButtonElement)) {
        return;
    }
    const dirty = [...row.querySelectorAll("[data-list-edit]")].some((item) => listValue(item) !== item.dataset.saved);
    button.hidden = !dirty;
}

document.addEventListener("click", async (event) => {
    const button = event.target instanceof Element ? event.target.closest("[data-row-save]") : null;
    if (!(button instanceof HTMLButtonElement)) {
        return;
    }
    const row = button.closest("tr");
    const wrap = button.closest("[data-model]");
    if (!(row instanceof HTMLTableRowElement) || !(wrap instanceof HTMLElement)) {
        return;
    }
    const data = new URLSearchParams();
    data.set("csrf_token", wrap.dataset.csrf || "");
    for (const control of row.querySelectorAll("[data-list-edit]")) {
        data.set(control.dataset.field || "", listValue(control));
    }
    button.disabled = true;
    const error = row.querySelector(".row-error");
    if (error instanceof HTMLElement) {
        error.hidden = true;
        error.textContent = "";
    }
    try {
        const response = await fetch(`/admin/models/${wrap.dataset.model}/${button.dataset.rowSave}/row`, {
            method: "POST",
            body: data,
            headers: { "Accept": "application/json" },
        });
        const body = await response.json();
        if (!response.ok || !body.ok) {
            if (error instanceof HTMLElement) {
                error.hidden = false;
                error.textContent = body.error || "";
            }
            return;
        }
        for (const control of row.querySelectorAll("[data-list-edit]")) {
            control.dataset.saved = listValue(control);
            const link = control.parentElement instanceof HTMLElement ? control.parentElement.querySelector("a") : null;
            if (link instanceof HTMLAnchorElement && control instanceof HTMLInputElement && control.type === "text") {
                link.textContent = control.value;
            }
        }
        button.hidden = true;
    } catch {
        if (error instanceof HTMLElement) {
            error.hidden = false;
        }
    } finally {
        button.disabled = false;
    }
});

document.addEventListener("scroll", () => {
    for (const menu of document.querySelectorAll("details.menu[open]")) {
        menu.open = false;
    }
}, true);
