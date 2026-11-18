const { defineConfig } = require("@playwright/test");

module.exports = defineConfig({
  testDir: ".",
  timeout: 30_000,
  use: {
    baseURL: process.env.FEXU_BROWSER_BASE_URL || "http://127.0.0.1:4173",
    ignoreHTTPSErrors: true,
  },
  projects: [
    {
      name: "chromium",
      testIgnore: /admin-relations\.spec\.js/,
      use: { browserName: "chromium" },
    },
    {
      name: "admin-relations",
      testMatch: /admin-relations\.spec\.js/,
      use: {
        browserName: "chromium",
        baseURL: process.env.FEXU_ADMIN_BROWSER_BASE_URL || "http://127.0.0.1:4174",
      },
    },
  ],
});
