import { query } from "@anthropic-ai/claude-agent-sdk";
import { tmpdir } from "node:os";
import { pathToFileURL } from "node:url";

const TIMEOUT_MS = 8000;

export async function collectClaudeModels({
	queryImpl = query,
	cwd = tmpdir(),
	timeoutMs = TIMEOUT_MS,
	AbortControllerImpl = AbortController,
} = {}) {
	const abort = new AbortControllerImpl();

	// Generator awaits forever; we only need the SDK initializationResult,
	// not an actual turn. Abort triggers cleanup on exit.
	const generator = (async function* () {
		await new Promise(() => {});
	})();

	const q = queryImpl({
		prompt: generator,
		options: {
			cwd,
			abortController: abort,
			permissionMode: "acceptEdits",
			settingSources: ["user"],
			pathToClaudeCodeExecutable: "claude",
		},
	});

	let timer;
	try {
		const result = await Promise.race([
			q.initializationResult(),
			new Promise((_, reject) => {
				timer = setTimeout(
					() => reject(new Error("initializationResult timeout")),
					timeoutMs,
				);
			}),
		]);
		return Array.isArray(result?.models) ? result.models : [];
	} finally {
		if (timer) clearTimeout(timer);
		abort.abort();
	}
}

export async function runClaudeListModelsProbe({
	writeStdout = (text) => process.stdout.write(text),
	writeStderr = (text) => process.stderr.write(text),
	exit = (code) => process.exit(code),
	...deps
} = {}) {
	try {
		const models = await collectClaudeModels(deps);
		writeStdout(`${JSON.stringify({ models })}\n`);
		exit(0);
	} catch (err) {
		const msg = err instanceof Error ? err.message : String(err);
		writeStderr(`claude-list-models failed: ${msg}\n`);
		exit(1);
	}
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
	await runClaudeListModelsProbe();
}
