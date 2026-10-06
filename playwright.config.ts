import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
 retries: process.env.CI ? 1 : 0,
 reporter: process.env.CI ? "github" : "list",
 use: { trace: "on-first-retry" },
 projects: [
  { name: "integration", testDir: "./tests/integration", use: { ...devices["Desktop Chrome"] } },
  { name: "behavior", testDir: "./tests/behavior", use: { ...devices["Desktop Chrome"] } },
 ],
});
