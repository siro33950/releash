import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
	testDir: "./tests/screenshots",
	testMatch: "*.screenshot.ts",
	snapshotDir: "./tests/__screenshots__",
	snapshotPathTemplate: "{snapshotDir}/{projectName}/{arg}{ext}",
	fullyParallel: false,
	retries: process.env.CI ? 1 : 0,
	workers: 1,
	reporter: process.env.CI
		? [["github"], ["html", { open: "never" }]]
		: "list",
	expect: {
		toHaveScreenshot: {
			maxDiffPixelRatio: 0.01,
			threshold: 0.2,
		},
	},
	use: {
		baseURL: "http://localhost:1420",
		viewport: { width: 1440, height: 900 },
		animations: "disabled",
		trace: "on-first-retry",
		screenshot: "only-on-failure",
	},
	projects: [
		{ name: "chromium", use: { ...devices["Desktop Chrome"] } },
	],
	webServer: {
		command: "pnpm dev",
		url: "http://localhost:1420",
		reuseExistingServer: !process.env.CI,
		timeout: 30_000,
	},
});
