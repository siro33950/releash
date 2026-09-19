import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import type { Options } from "@wdio/types";
import { config as performanceConfig } from "./wdio.performance.conf";

const directory = process.env.RELEASH_CLIENT_STREAMS_DIRECTORY;
if (!directory) throw new Error("Run with pnpm test:client-streams:macos");
const appBinaryPath = "./src-tauri/target/debug/releash";

export const config: Options.Testrunner = {
	...performanceConfig,
	specs: ["./tests/tauri-performance/client-streams.spec.ts"],
	services: [
		[
			"@wdio/tauri-service",
			{
				appBinaryPath,
				driverProvider: "embedded",
				captureBackendLogs: true,
				captureFrontendLogs: true,
				env: {
					SHELL: "/bin/bash",
					RELEASH_PERF_REAL_APP: "1",
					RELEASH_PERFORMANCE_DATA_DIR: join(directory, "data"),
					RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE: resolve(
						"tests/fixtures/terminal-launch-provider-fixture",
					),
				},
			},
		],
	],
	capabilities: [
		{
			browserName: "tauri",
			"tauri:options": { application: appBinaryPath },
			"wdio:tauriServiceOptions": { windowLabel: "main" },
		},
	],
	onPrepare() {
		if (process.platform !== "darwin")
			throw new Error("This acceptance test requires macOS WKWebView");
		execFileSync("python3", ["--version"]);
		mkdirSync(join(directory, "data"));
		writeFileSync(
			join(directory, "data/releash.toml"),
			"[telemetry]\ncrash_reporting = false\nperformance_telemetry = false\n",
		);
		for (let index = 0; index < 6; index++) {
			const path = join(directory, `pane-${index}`);
			execFileSync("git", ["init", "--initial-branch", `pane-${index}`, path]);
			execFileSync("git", [
				"-C",
				path,
				"-c",
				"user.name=Acceptance",
				"-c",
				"user.email=acceptance@example.invalid",
				"-c",
				"commit.gpgsign=false",
				"commit",
				"--allow-empty",
				"-m",
				"fixture",
			]);
		}
	},
	mochaOpts: { ui: "bdd", timeout: 180_000 },
};
