import type { MessagePart } from "@/types/session";

export interface ToolPairingResult {
	pairedResults: Map<number, Extract<MessagePart, { type: "tool_result" }>>;
	skippedResultIndices: Set<number>;
}

export function buildToolPairings(parts: MessagePart[]): ToolPairingResult {
	const pairedResults = new Map<
		number,
		Extract<MessagePart, { type: "tool_result" }>
	>();
	const skippedResultIndices = new Set<number>();

	// Build ID-based result map: toolUseId → { index, result }
	const resultByToolUseId = new Map<
		string,
		{ index: number; result: Extract<MessagePart, { type: "tool_result" }> }
	>();
	for (let i = 0; i < parts.length; i++) {
		const p = parts[i];
		if (p.type === "tool_result" && p.toolUseId) {
			resultByToolUseId.set(p.toolUseId, {
				index: i,
				result: p,
			});
		}
	}

	// Pair tool_use with tool_result (ID-based first, then adjacent fallback)
	for (let i = 0; i < parts.length; i++) {
		const part = parts[i];
		if (part.type !== "tool_use") continue;

		// ID-based pairing
		const byId = resultByToolUseId.get(part.id);
		if (byId) {
			pairedResults.set(i, byId.result);
			skippedResultIndices.add(byId.index);
			continue;
		}

		// Adjacent fallback: next part is an unpaired tool_result
		const next = parts[i + 1];
		if (next?.type === "tool_result" && !skippedResultIndices.has(i + 1)) {
			pairedResults.set(i, next);
			skippedResultIndices.add(i + 1);
		}
	}

	return { pairedResults, skippedResultIndices };
}
