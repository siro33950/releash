import { query } from "@anthropic-ai/claude-agent-sdk";
import crypto from "node:crypto";
import { buildSystemPromptOption } from "./bridge-utils.mjs";

const pendingPermissions = new Map();
const messageQueue = [];
let currentQuery = null;
let currentAbortController = null;
let currentSessionId = null;
let messageResolve = null;
let closed = false;
let sessionReady = false;
let currentModelId = null;
let pendingRestoreContext = null;

/**
 * AsyncGenerator that yields prompts to the SDK.
 * Each yield triggers a new turn. The SDK processes the prompt and emits
 * messages via the query iterator. After a turn completes, the SDK waits
 * for the next yield.
 *
 * IMPORTANT: Do NOT close this generator while the SDK is processing (#9705).
 * Only return (close) on explicit "close" command after the turn has completed.
 */
async function* promptGenerator() {
	while (!closed) {
		if (messageQueue.length > 0) {
			yield messageQueue.shift();
			continue;
		}
		const prompt = await new Promise((resolve) => {
			messageResolve = resolve;
		});
		messageResolve = null;
		if (prompt === null) return;
		yield prompt;
	}
}

function emit(obj) {
	process.stdout.write(JSON.stringify(obj) + "\n");
}

let stdinBuffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
	stdinBuffer += chunk;
	const lines = stdinBuffer.split("\n");
	stdinBuffer = lines.pop();
	for (const line of lines) {
		if (!line.trim()) continue;
		try {
			const cmd = JSON.parse(line);
			handleCommand(cmd);
		} catch (e) {
			process.stderr.write(
				`bridge: stdin parse error: ${e instanceof Error ? e.message : String(e)}\n`,
			);
		}
	}
});

/**
 * Wrap a plain text prompt into an SDKUserMessage that the CLI's
 * `--input-format stream-json` protocol expects.
 */
function toUserMessage(text, images) {
	const content = [];
	if (images && images.length > 0) {
		for (const img of images) {
			content.push({
				type: "image",
				source: {
					type: "base64",
					media_type: img.mediaType,
					data: img.data,
				},
			});
		}
	}
	if (text) {
		content.push({ type: "text", text });
	}
	return {
		type: "user",
		message: {
			role: "user",
			content,
		},
		parent_tool_use_id: null,
		session_id: "",
	};
}

let currentPermissionMode = "acceptEdits";

function applyRestoreContext(text) {
	const prefix = pendingRestoreContext?.promptPrefix;
	pendingRestoreContext = null;
	if (!prefix || typeof prefix !== "string" || prefix.trim().length === 0) {
		return text;
	}
	if (!text) return prefix;
	return `${prefix}\n\n${text}`;
}

function applyModelSafely(modelId) {
	try {
		currentQuery?.setModel(modelId ?? undefined);
	} catch (e) {
		process.stderr.write(
			`bridge: setModel failed: ${e instanceof Error ? e.message : String(e)}\n`,
		);
		emit({
			type: "error",
			message: `Failed to apply model: ${e instanceof Error ? e.message : String(e)}`,
		});
	}
}

function handleCommand(cmd) {
	switch (cmd.type) {
		case "init":
			handleInit(cmd);
			break;
		case "message": {
			const msg = toUserMessage(applyRestoreContext(cmd.prompt), cmd.images);
			if (messageResolve) {
				messageResolve(msg);
			} else {
				messageQueue.push(msg);
			}
			break;
		}
		case "interrupt":
			if (currentAbortController) {
				currentAbortController.abort();
			}
			break;
		case "permission_response": {
			const pending = pendingPermissions.get(cmd.request_id);
			if (pending) {
				pendingPermissions.delete(cmd.request_id);
				const result = cmd.result;
				if (result.behavior === "allow" && !result.updatedInput) {
					result.updatedInput = pending.input;
				}
				pending.resolve(result);
			}
			break;
		}
		case "setMode": {
			currentPermissionMode = cmd.permissionMode || "acceptEdits";
			if (currentQuery) {
				currentQuery.setPermissionMode(currentPermissionMode);
			}
			break;
		}
		case "setModel": {
			currentModelId = cmd.modelId || null;
			applyModelSafely(cmd.modelId || null);
			break;
		}
		case "close":
			closed = true;
			if (messageResolve) {
				messageResolve(null);
				messageResolve = null;
			}
			break;
	}
}

async function handleInit(cmd) {
	const permissionMode = cmd.permissionMode || "acceptEdits";
	currentPermissionMode = permissionMode;
	const promptPrefix = cmd.restoreContext?.promptPrefix;
	pendingRestoreContext =
		typeof promptPrefix === "string" && promptPrefix.trim()
			? { promptPrefix }
			: null;

	let stderrChunks = [];
	const options = {
		cwd: cmd.cwd,
		permissionMode,
		includePartialMessages: true,
		settingSources: ["user", "project"],
		pathToClaudeCodeExecutable: "claude",
		stderr: (data) => {
			stderrChunks.push(data);
		},
		...buildSystemPromptOption(cmd.systemPrompt),
	};

	if (permissionMode === "bypassPermissions") {
		options.allowDangerouslySkipPermissions = true;
	}

	if (cmd.model) {
		options.model = cmd.model;
	}

	const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];

	options.canUseTool = (toolName, input, meta) => {
		if (
			currentPermissionMode === "bypassPermissions" ||
			(currentPermissionMode !== "default" && !INTERACTIVE_TOOLS.includes(toolName))
		) {
			return { behavior: "allow", updatedInput: input };
		}
		return new Promise((resolve) => {
			const requestId = crypto.randomUUID();
			pendingPermissions.set(requestId, { resolve, input });
			emit({
				type: "permission_request",
				request_id: requestId,
				tool_name: toolName,
				input,
				tool_use_id: meta.toolUseID,
				title: meta.title,
				display_name: meta.displayName,
				description: meta.description,
				decision_reason: meta.decisionReason,
			});
		});
	};

	if (cmd.sessionId) {
		options.resume = cmd.sessionId;
	}

	while (!closed) {
		stderrChunks = [];
		currentAbortController = new AbortController();
		options.abortController = currentAbortController;

		if (currentSessionId) {
			options.resume = currentSessionId;
		}

		messageResolve = null;
		const generator = promptGenerator();
		currentQuery = query({ prompt: generator, options });

		if (!sessionReady) {
			currentQuery
				.supportedCommands()
				.then((commands) => {
					emit({ type: "supported_commands", commands });
				})
				.catch((e) => {
					process.stderr.write(
						`bridge: supportedCommands failed: ${e instanceof Error ? e.message : String(e)}\n`,
					);
				});
		}

		if (currentModelId) {
			applyModelSafely(currentModelId);
		}

		let gotResult = false;
		try {
			for await (const message of currentQuery) {
				// New turn started after a previous result — reset per-turn flag
				if (gotResult && message.type !== "result") {
					gotResult = false;
				}

				if (message.session_id && message.session_id !== currentSessionId) {
					currentSessionId = message.session_id;
					emit({ type: "session_ready", session_id: message.session_id });
					sessionReady = true;
				}
				emit(message);

				if (message.type === "result") {
					gotResult = true;
					const hasErrors =
						message.errors && Array.isArray(message.errors) && message.errors.length > 0;
					emit({
						type: "turn_complete",
						session_id: message.session_id || null,
						exit_code: hasErrors ? 1 : 0,
					});
				}
			}
			// abort が throw せず正常完了した場合もループを継続
			if (currentAbortController.signal.aborted) {
				pendingPermissions.clear();
				if (!gotResult) {
					emit({
						type: "turn_complete",
						session_id: currentSessionId || null,
						exit_code: 0,
					});
				}
				continue;
			}
			break;
		} catch (e) {
			if (currentAbortController?.signal?.aborted) {
				pendingPermissions.clear();
				if (!gotResult) {
					emit({
						type: "turn_complete",
						session_id: currentSessionId || null,
						exit_code: 0,
					});
				}
				continue;
			}
			const stderrText = stderrChunks.join("").trim();
			emit({
				type: "error",
				message: e instanceof Error ? e.message : String(e),
				stderr: stderrText || undefined,
			});
			break;
		}
	}

	process.exit(0);
}
