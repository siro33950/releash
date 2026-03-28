import { query } from "@anthropic-ai/claude-agent-sdk";
import crypto from "node:crypto";

const pendingPermissions = new Map();
const messageQueue = [];
let currentQuery = null;
let messageResolve = null;
let closed = false;
let sessionReady = false;

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
function toUserMessage(text) {
	return {
		type: "user",
		message: {
			role: "user",
			content: [{ type: "text", text }],
		},
		parent_tool_use_id: null,
		session_id: "",
	};
}

function handleCommand(cmd) {
	switch (cmd.type) {
		case "init":
			handleInit(cmd);
			break;
		case "message": {
			const msg = toUserMessage(cmd.prompt);
			if (messageResolve) {
				messageResolve(msg);
			} else {
				messageQueue.push(msg);
			}
			break;
		}
		case "interrupt":
			if (currentQuery) {
				currentQuery.interrupt();
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
	};

	if (permissionMode === "bypassPermissions") {
		options.allowDangerouslySkipPermissions = true;
	}

	const INTERACTIVE_TOOLS = ["AskUserQuestion", "EnterPlanMode", "ExitPlanMode"];

	if (permissionMode !== "bypassPermissions") {
		options.canUseTool = (toolName, input, meta) => {
			if (permissionMode !== "default" && !INTERACTIVE_TOOLS.includes(toolName)) {
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
	}

	if (cmd.sessionId) {
		options.resume = cmd.sessionId;
	}

	const generator = promptGenerator();
	currentQuery = query({ prompt: generator, options });

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

	try {
		for await (const message of currentQuery) {
			if (!sessionReady && message.session_id) {
				emit({ type: "session_ready", session_id: message.session_id });
				sessionReady = true;
			}
			emit(message);

			if (message.type === "result") {
				const hasErrors =
					message.errors && Array.isArray(message.errors) && message.errors.length > 0;
				emit({
					type: "turn_complete",
					session_id: message.session_id || null,
					exit_code: hasErrors ? 1 : 0,
				});
			}
		}
	} catch (e) {
		const stderrText = stderrChunks.join("").trim();
		emit({
			type: "error",
			message: e instanceof Error ? e.message : String(e),
			stderr: stderrText || undefined,
		});
	}

	process.exit(0);
}
