const { test, expect } = require("@playwright/test");

const PASSWORD = process.env.FEXU_BROWSER_ADMIN_PASSWORD;
if (!PASSWORD) {
  throw new Error("FEXU_BROWSER_ADMIN_PASSWORD is required");
}
const STAFF = "staff@example.test";
const READER = "reader@example.test";

test.describe.configure({ mode: "serial" });

test.beforeEach(async ({ page }) => {
  await page.goto("/admin/login");
});

async function login(page, email) {
  await page.locator("#email").fill(email);
  await page.locator("#password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.waitForURL(/\/admin\/?$/);
}

test("login labels, keyboard focus and a narrow viewport stay usable without a script", async ({
  browser,
}) => {
  const context = await browser.newContext({
    javaScriptEnabled: false,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  await page.goto("/admin/login");
  await expect(page.getByLabel("Email")).toBeVisible();
  await expect(page.getByLabel("Password")).toBeVisible();
  await page.locator("#email").focus();
  await page.keyboard.press("Tab");
  await expect(page.locator("#password")).toBeFocused();
  await page.locator("#email").fill("nobody@example.test");
  await page.locator("#password").fill("not-a-real-password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.locator("#email")).toHaveValue("nobody@example.test");
  await expect(page.locator('input[name="password"]')).toHaveCount(1);
  const cookies = await context.cookies();
  expect(cookies.find((cookie) => cookie.name === "fexu_session")).toBeUndefined();
  await context.close();

  const focused = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const visible = await focused.newPage();
  await visible.goto("/admin/login");
  await visible.locator("#email").focus();
  await visible.keyboard.press("Tab");
  await expect(visible.locator("#password")).toBeFocused();
  const outline = await visible.locator("#password").evaluate((node) => {
    return window.getComputedStyle(node).outlineWidth;
  });
  expect(outline).not.toBe("0px");
  const overflow = await visible.evaluate(() => {
    return document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1;
  });
  expect(overflow).toBe(true);
  await focused.close();
});

test("a staff user pages, searches and keeps a foreign shelf out of the picker", async ({
  page,
}) => {
  await login(page, STAFF);
  const picker = await page.goto("/admin/models/items/pick/shelf_id?page=2");
  expect(picker.status()).toBe(200);
  await expect(page.getByRole("link", { name: "Regal 25" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Regal 00" })).toHaveCount(0);
  await expect(page.getByText("Fremd-Regal")).toHaveCount(0);
  await expect(page.locator("body")).toContainText("Total");
  await expect(page.locator("body")).not.toContainText("Total: 31");
  const pickerSearch = page.locator("main form.filters");
  await pickerSearch.locator('input[name="q"]').fill("Regal 20");
  await pickerSearch.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("link", { name: "Regal 20" })).toBeVisible();
  await expect(page.getByText("Fremd-Regal")).toHaveCount(0);
  const hidden = await page.goto("/admin/models/items/pick/shelf_id?q=Fremd");
  expect(hidden.status()).toBe(200);
  await expect(page.getByText("Fremd-Regal")).toHaveCount(0);
  await expect(page.locator("ul.choice-list a")).toHaveCount(0);
});

test("foreign key, one-to-one and many-to-many edits keep an off-page selection", async ({
  page,
}) => {
  await login(page, STAFF);
  await page.goto("/admin/models/items");
  await page.getByRole("link", { name: "Gewählt" }).click();
  await expect(page.locator("#shelf_id option:checked")).toHaveText("Regal 20");
  const currentDesk = (await page.locator("#desk_id option:checked").textContent()).trim();
  const nextDesk = currentDesk === "Pult A" ? "Pult B" : "Pult A";
  const tags = page.locator("#m2m-tags");
  await expect(tags).toBeVisible();
  await expect(tags.getByRole("checkbox")).toHaveCount(100);
  await expect(tags.getByRole("checkbox", { name: "Tag 100" })).toHaveCount(0);
  await expect(tags.locator('input[type="hidden"][name="m2m.tags"]')).toHaveCount(1);
  await expect(tags).toContainText("Selected records on other pages stay selected.");
  await Promise.all([page.waitForEvent("load"), tags.getByRole("button", { name: "Next" }).click()]);
  await expect(tags.getByRole("checkbox", { name: "Tag 100" })).toBeChecked();
  await Promise.all([page.waitForEvent("load"), tags.getByRole("button", { name: "Previous" }).click()]);
  await expect(tags.locator('input[type="hidden"][name="m2m.tags"]')).toHaveCount(1);
  await page.locator("#m2m-tags-q").fill("Tag 100");
  await Promise.all([page.waitForEvent("load"), tags.getByRole("button", { name: "Search" }).click()]);
  await expect(tags.getByRole("checkbox", { name: "Tag 100" })).toBeChecked();
  await expect(page.locator("form[data-track-dirty=record]")).toHaveAttribute("data-dirty", "0");
  await page.locator("#desk_id").selectOption({ label: nextDesk });
  await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "Save and continue editing" }).click()]);
  await expect(page.locator("#desk_id option:checked")).toHaveText(nextDesk);
  await expect(page.locator("#shelf_id option:checked")).toHaveText("Regal 20");
  await expect(tags).toContainText("Total: 1");
  await expect(tags.locator('input[type="hidden"][name="m2m.tags"]')).toHaveCount(1);
});

test("an invalid item redisplays the typed name and the selected shelf", async ({ page }) => {
  await login(page, STAFF);
  await page.goto("/admin/models/items/new");
  await page.locator("#name").fill("ab");
  await page.locator("#shelf_id").selectOption({ label: "Regal 01" });
  await page.locator("div.actions button[value='list']").click();
  await expect(page).toHaveURL(/\/admin\/models\/items\/new$/);
  await expect(page.locator("#name")).toHaveValue("ab");
  await expect(page.locator("#shelf_id option:checked")).toHaveText("Regal 01");
  await expect(page.locator("#form-errors")).toHaveAttribute("role", "alert");
  await expect(page.locator("#name-error")).toContainText("too short");
});

test("a forged shelf id is rejected and does not create the item", async ({ page }) => {
  await login(page, STAFF);
  await page.goto("/admin/models/items/new");
  const csrf = await page.locator('#content input[name="csrf_token"]').inputValue();
  const response = await page.request.post("/admin/models/items/new", {
    form: {
      csrf_token: csrf,
      name: "Forged shelf",
      shelf_id: "00000000-0000-0000-0000-000000000099",
      active: "true",
      save: "detail",
    },
    maxRedirects: 0,
  });
  expect(response.status()).not.toBe(303);
  const body = await response.text();
  expect(body).toContain("Forged shelf");
  expect(body).toContain("Choose a visible record.");
  expect(body).toContain("Not visible");
  expect(body).not.toContain("Fremd-Regal");
  await page.goto("/admin/models/items?q=Forged");
  await expect(page.getByRole("link", { name: "Forged shelf" })).toHaveCount(0);
});

test("inline create, update and delete stay on the project form", async ({ page }) => {
  await login(page, STAFF);
  await page.goto("/admin/models/projects/new");
  await page.locator("#name").fill("Auftrag");
  await page.locator('input[name="inline_tasks_0_label"]').fill("Erste Zeile");
  await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "Save and continue editing" }).click()]);
  await expect(page.locator('input[name="inline_tasks_0_label"]')).toHaveValue("Erste Zeile");
  await page.locator('input[name="inline_tasks_0_label"]').fill("Geändert");
  await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "Save and continue editing" }).click()]);
  await expect(page.locator('input[name="inline_tasks_0_label"]')).toHaveValue("Geändert");
  await page.locator('input[name="inline_tasks_0_remove"]').check();
  await Promise.all([page.waitForEvent("load"), page.getByRole("button", { name: "Save and continue editing" }).click()]);
  await expect(page.locator('input[name="inline_tasks_0_label"]')).toHaveValue("");
});

test("bulk deactivation asks for confirmation and a reader cannot apply it", async ({
  page,
  browser,
}) => {
  await login(page, STAFF);
  await page.goto("/admin/models/items");
  await page.getByRole("checkbox", { name: "Select Bulk A" }).check();
  await page.getByRole("checkbox", { name: "Select Bulk B" }).check();
  await page.locator("#bulk-action").selectOption("deactivate");
  await page.getByRole("button", { name: "Run" }).click();
  await expect(page.locator("#bulk-heading")).toHaveText("Confirm selection");
  await expect(page.locator("#content input[name='csrf_token']")).toHaveValue(/.+/);
  await expect(page.locator("#content input[name='confirm']")).toHaveValue("1");
  const forged = await page.request.post("/admin/models/items/bulk", {
    form: { action: "deactivate", ids: "00000000-0000-0000-0000-000000000099" },
    maxRedirects: 0,
  });
  expect(forged.status()).toBe(403);
  await page.getByRole("button", { name: "Confirm" }).click();
  await expect(page.getByRole("link", { name: "Bulk A" })).toBeVisible();

  const readerContext = await browser.newContext();
  const reader = await readerContext.newPage();
  await reader.goto("/admin/login");
  await reader.locator("#email").fill(READER);
  await reader.locator("#password").fill(PASSWORD);
  await reader.getByRole("button", { name: "Sign in" }).click();
  await reader.waitForURL(/\/admin\/?$/);
  await reader.goto("/admin/models/items");
  const token = await reader.locator('input[name="csrf_token"]').first().inputValue();
  const denied = await reader.request.post("/admin/models/items/bulk", {
    form: {
      csrf_token: token,
      action: "delete",
      ids: "00000000-0000-0000-0000-000000000001",
    },
    maxRedirects: 0,
  });
  expect(denied.status()).toBe(403);
  await readerContext.close();
});
