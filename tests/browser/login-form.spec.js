const { test, expect } = require("@playwright/test");

test("fresh profile submits the login form and receives no session", async ({ browser }) => {
  const context = await browser.newContext({ storageState: undefined });
  const page = await context.newPage();
  const response = await page.goto("/admin/login");
  expect(response.status()).toBe(200);
  await page.locator("#email").fill("nobody@example.com");
  await page.locator("#password").fill("not-a-real-password");
  await page.locator("button[type=submit]").click();
  await page.waitForLoadState("domcontentloaded");
  const cookies = await context.cookies();
  expect(cookies.find((cookie) => cookie.name === "fexu_session")).toBeUndefined();
  await expect(page.locator('input[type="password"]')).toBeVisible();
  await context.close();
});
