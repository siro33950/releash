import type { ChatMessage, TurnPhase } from "@/types/session";

export type ActivityStatus = { label: string } | null;

/**
 * session の最新メッセージ + 現在 turnPhase からアクティビティ表示文言を派生する純粋関数。
 * useAgentChat 内部 / BoundSessionChat の双方から利用される。
 */
export function deriveActivityStatus(
	messages: ChatMessage[] | undefined,
	turnPhase: TurnPhase,
): ActivityStatus {
	if (turnPhase === "idle") return null;
	if (!messages || messages.length === 0) return null;
	const lastMsg = messages[messages.length - 1];
	if (lastMsg.role !== "agent") return null;
	if (lastMsg.parts.length === 0) return { label: "Thinking..." };

	const lastPart = lastMsg.parts[lastMsg.parts.length - 1];
	switch (lastPart.type) {
		case "thinking":
			return { label: "Thinking..." };
		case "text":
			return { label: "Writing..." };
		case "tool_use": {
			const tool = (
				lastPart as { tool: string; input?: Record<string, unknown> }
			).tool;
			const filePath = (lastPart as { input?: Record<string, unknown> }).input
				?.file_path as string | undefined;
			const fileName = filePath?.split("/").pop();
			switch (tool) {
				case "Read":
					return {
						label: fileName ? `Reading ${fileName}` : "Reading file...",
					};
				case "Write":
					return {
						label: fileName ? `Writing ${fileName}` : "Writing file...",
					};
				case "Edit":
					return {
						label: fileName ? `Editing ${fileName}` : "Editing file...",
					};
				case "Bash":
					return { label: "Running command..." };
				case "Grep":
					return { label: "Searching..." };
				case "Glob":
					return { label: "Finding files..." };
				case "Task":
					return { label: "Running background task..." };
				case "WebFetch":
					return { label: "Fetching web content..." };
				case "WebSearch":
					return { label: "Searching the web..." };
				default:
					return { label: `Using ${tool}...` };
			}
		}
		case "tool_result":
			return { label: "Processing result..." };
		case "permission":
			return { label: "Waiting for permission..." };
		case "task_status":
			return { label: "Running background task..." };
		case "error":
			return null;
		default:
			return { label: "Working..." };
	}
}
