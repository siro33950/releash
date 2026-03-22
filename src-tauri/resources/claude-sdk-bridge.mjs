import { query } from "@anthropic-ai/claude-agent-sdk";
import crypto from "node:crypto";

const args = JSON.parse(process.argv[2]);
const permissionMode = args.permissionMode || "acceptEdits";

const pendingPermissions = new Map();

const options = {
	cwd: args.cwd,
	permissionMode,
	includePartialMessages: true,
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
			process.stdout.write(
				JSON.stringify({
					type: "permission_request",
					request_id: requestId,
					tool_name: toolName,
					input,
					tool_use_id: meta.toolUseID,
					title: meta.title,
					display_name: meta.displayName,
					description: meta.description,
					decision_reason: meta.decisionReason,
				}) + "\n",
			);
		});
	};
}

if (args.sessionId) {
	options.resume = args.sessionId;
}

const q = query({ prompt: args.prompt, options });

let stdinBuffer = "";
process.stdin.on("data", (chunk) => {
	stdinBuffer += chunk.toString();
	const lines = stdinBuffer.split("\n");
	stdinBuffer = lines.pop();
	for (const line of lines) {
		if (!line.trim()) continue;
		try {
			const cmd = JSON.parse(line);
			if (cmd.type === "interrupt") {
				q.interrupt();
			} else if (cmd.type === "permission_response") {
				const pending = pendingPermissions.get(cmd.request_id);
				if (pending) {
					pendingPermissions.delete(cmd.request_id);
					const result = cmd.result;
					if (result.behavior === "allow" && !result.updatedInput) {
						result.updatedInput = pending.input;
					}
					pending.resolve(result);
				}
			}
		} catch (e) {
			process.stderr.write(`bridge: stdin parse error: ${e instanceof Error ? e.message : String(e)}\n`);
		}
	}
});

try {
	for await (const message of q) {
		process.stdout.write(JSON.stringify(message) + "\n");
	}
} catch (e) {
	process.stdout.write(
		JSON.stringify({
			type: "result",
			errors: [e instanceof Error ? e.message : String(e)],
		}) + "\n",
	);
	process.exit(1);
}
process.exit(0);
