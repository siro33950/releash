import type { Options } from "@wdio/types";
import { resolve } from "node:path";

export const appBinaryPath = "./src-tauri/target/release/releash";
const launchProvider = process.env.RELEASH_PERFORMANCE_LAUNCH_PROVIDER;
const realAppMode = process.env.RELEASH_PERFORMANCE_REAL_APP === "1";

const killSwitchPassthrough = Object.fromEntries(
	Object.entries(process.env).filter(
		(entry): entry is [string, string] =>
			entry[0].startsWith("RELEASH_PERF_") && entry[1] !== undefined,
	),
);
const appEnvironment: Record<string, string> = {
	...killSwitchPassthrough,
	// ユーザーのinteractive shell設定（zsh autosuggestions・共有履歴等）は
	// echo内容を非決定にするため、harnessのPTYはplain bashに固定する。
	SHELL: "/bin/bash",
};
if (launchProvider === "fixture") {
	appEnvironment.RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE = resolve(
		"tests/fixtures/terminal-launch-provider-fixture",
	);
}
if (launchProvider === "tui-fixture") {
	appEnvironment.RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE = resolve(
		"tests/fixtures/terminal-launch-tui-fixture",
	);
}
if (realAppMode) {
	appEnvironment.RELEASH_PERF_REAL_APP = "1";
}

export const config: Options.Testrunner = {
	runner: "local",
	specs: realAppMode
		? ["./tests/tauri-performance/terminal-real-app-load.spec.ts"]
		: launchProvider
			? ["./tests/tauri-performance/terminal-launch-real-ipc.spec.ts"]
			: ["./tests/tauri-performance/terminal-real-ipc.spec.ts"],
	maxInstances: 1,
	services: [
		[
			"@wdio/tauri-service",
			{
				appBinaryPath,
				driverProvider: "embedded",
				captureBackendLogs: true,
				captureFrontendLogs: true,
				env: Object.keys(appEnvironment).length ? appEnvironment : undefined,
			},
		],
	],
	capabilities: [
		{
			browserName: "tauri",
			"tauri:options": { application: appBinaryPath },
		},
	],
	logLevel: "warn",
	bail: 1,
	waitforTimeout: 15_000,
	connectionRetryTimeout: 90_000,
	connectionRetryCount: 1,
	framework: "mocha",
	reporters: ["spec"],
	mochaOpts: {
		ui: "bdd",
		timeout: 900_000,
	},
};
