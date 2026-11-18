const { test, expect } = require("@playwright/test");

test("login stays labeled and focused on a narrow screen", async ({ browser }) => {
  const context = await browser.newContext({
    storageState: undefined,
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  const response = await page.goto("/admin/login");
  expect(response.status()).toBe(200);
  const email = page.locator("#email");
  const password = page.locator("#password");
  await expect(page.locator('label[for="email"]')).toBeVisible();
  await expect(page.locator('label[for="password"]')).toBeVisible();
  await email.focus();
  await page.keyboard.press("Tab");
  await expect(password).toBeFocused();
  const outlineWidth = await password.evaluate((node) => getComputedStyle(node).outlineWidth);
  expect(outlineWidth).not.toBe("0px");
  await context.close();
});

test("an anonymous relation picker does not return records", async ({ browser }) => {
  const context = await browser.newContext({ storageState: undefined });
  const page = await context.newPage();
  const response = await page.goto("/admin/models/products/pick/category_id");
  expect(response.status()).toBeLessThan(500);
  const body = await page.locator("body").innerText();
  expect(body).not.toContain("row-0000");
  await expect(page.locator("ul.choice-list a")).toHaveCount(0);
  await context.close();
});
