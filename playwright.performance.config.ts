import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
	testDir: "./tests",
	testMatch: "terminal-performance.spec.ts",
	fullyParallel: false,
	retries: 0,
	workers: 1,
	reporter: "list",
	use: {
		baseURL: "http://127.0.0.1:1421",
		trace: "retain-on-failure",
		video: "retain-on-failure",
		screenshot: "only-on-failure",
	},
	projects: [
		{ name: "chromium", use: { ...devices["Desktop Chrome"] } },
	],
	webServer: {
		command: "pnpm preview --host 127.0.0.1 --port 1421",
		url: "http://127.0.0.1:1421",
		reuseExistingServer: false,
		timeout: 30_000,
	},
});
