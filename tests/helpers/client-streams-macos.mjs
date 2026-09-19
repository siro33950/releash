import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout } from "node:timers/promises";

if (process.platform !== "darwin")
	throw new Error("This acceptance test requires macOS WKWebView");
const directory = await realpath(
	await mkdtemp("/private/tmp/releash-client-streams-"),
);
try {
	const child = spawn(
		"pnpm",
		["exec", "wdio", "run", "wdio.client-streams.conf.ts"],
		{
			stdio: "inherit",
			env: { ...process.env, RELEASH_CLIENT_STREAMS_DIRECTORY: directory },
		},
	);
	process.exitCode = await new Promise((resolve, reject) => {
		child.once("error", reject);
		child.once("exit", (code) => resolve(code ?? 1));
	});
} finally {
	const deadline = Date.now() + 15_000;
	while (existsSync(join(directory, "data/client-api.json"))) {
		try {
			const discovery = JSON.parse(
				await readFile(join(directory, "data/client-api.json"), "utf8"),
			);
			process.kill(discovery.pid, 0);
		} catch (error) {
			if (error.code === "ENOENT" || error.code === "ESRCH") break;
			throw error;
		}
		if (Date.now() >= deadline)
			throw new Error(`Test daemon has not stopped; retained ${directory}`);
		await setTimeout(100);
	}
	await rm(directory, { recursive: true, force: true });
}
