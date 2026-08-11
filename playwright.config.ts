import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
	testDir: "./tests",
	testIgnore: ["terminal-performance.spec.ts", "tauri-performance/**"],
	fullyParallel: false,
	retries: process.env.CI ? 1 : 0,
	workers: 1,
	reporter: process.env.CI
		? [["github"], ["html", { open: "never" }]]
		: "list",
	use: {
		baseURL: "http://localhost:1420",
		trace: "on-first-retry",
		video: "retain-on-failure",
		screenshot: "only-on-failure",
	},
	projects: [
		{ name: "chromium", use: { ...devices["Desktop Chrome"] } },
	],
	webServer: {
		command: "pnpm dev",
		url: "http://localhost:1420",
		reuseExistingServer: !process.env.CI,
		timeout: 120_000,
	},
});
