const { test, expect } = require("@playwright/test");

test("swagger ui renders the same-origin OpenAPI document", async ({ page }) => {
  const leaked = [];
  page.on("request", (request) => {
    const url = request.url();
    if (/unpkg|jsdelivr|cdnjs|validator\.swagger\.io|petstore/.test(url)) {
      leaked.push(url);
    }
  });
  const response = await page.goto("/api/docs");
  expect(response.status()).toBe(200);
  await expect(page.locator("#swagger-ui")).toBeVisible();
  await expect(page.locator(".opblock-summary-path", { hasText: "/healthz" })).toBeVisible({
    timeout: 20000,
  });
  await expect(page.getByRole("button", { name: "Try it out" })).toHaveCount(0);
  const spec = await page.request.get("/api/openapi.json");
  expect(spec.status()).toBe(200);
  const document = await spec.json();
  expect(document.openapi).toBe("3.1.0");
  const paths = JSON.stringify(document.paths);
  expect(paths).not.toContain('"/admin');
  expect(leaked).toEqual([]);
});
