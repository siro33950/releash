import { Codex } from "@openai/codex-sdk";
import { startCodexBridge } from "./codex-sdk-bridge-runtime.mjs";

export function createCodexSdk(cliPath = "codex", CodexImpl = Codex) {
	return new CodexImpl({
		codexPathOverride: cliPath || "codex",
	});
}

export function startCodexSdkBridge(options = {}) {
	return startCodexBridge({
		...options,
		codexFactory: ({ cliPath }) =>
			createCodexSdk(cliPath, options.CodexImpl ?? Codex),
	});
}
