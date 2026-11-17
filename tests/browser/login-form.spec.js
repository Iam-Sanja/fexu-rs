const { test, expect } = require("@playwright/test");

// Myshop in the browser job has no auth tables and does not call
// require_browser_writes(). A submitted login therefore cannot reach the
// credential form: the server answers 403 with the error document and sets
// no session. The migrated contract in examples/contract is the 401 path.
test("fresh profile submits the login form and receives no session", async ({ browser }) => {
  const context = await browser.newContext({ storageState: undefined });
  const page = await context.newPage();
  const response = await page.goto("/admin/login");
  expect(response.status()).toBe(200);
  await expect(page.locator('input[type="password"]')).toBeVisible();
  await page.locator("#email").fill("nobody@example.com");
  await page.locator("#password").fill("not-a-real-password");
  const submitted = page.waitForResponse(
    (reply) =>
      reply.url().endsWith("/admin/login") && reply.request().method() === "POST",
  );
  await page.locator("button[type=submit]").click();
  const post = await submitted;
  expect(post.status()).toBe(403);
  expect(post.headers()["content-type"]).toContain("text/html");
  expect(post.headers()["location"]).toBeUndefined();
  const setCookie = post.headers()["set-cookie"] ?? "";
  expect(setCookie).not.toContain("fexu_session=");
  await page.waitForLoadState("domcontentloaded");
  await expect(page).toHaveURL(/\/admin\/login$/);
  const cookies = await context.cookies();
  expect(cookies.find((cookie) => cookie.name === "fexu_session")).toBeUndefined();
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
  await expect(page.locator("p.alert")).toContainText(/not allowed|nicht erlaubt/);
  await context.close();
});
