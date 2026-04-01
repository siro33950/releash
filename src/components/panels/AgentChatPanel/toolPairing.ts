import type { MessagePart } from "@/types/session";

export interface TaskGroup {
	toolUseIndex: number;
	toolUseId: string;
	description?: string;
	subagentType?: string;
	childIndices: number[];
	statusParts: Extract<MessagePart, { type: "task_status" }>[];
	resultIndex?: number;
	isCompleted: boolean;
	isBackground: boolean;
	completionStatusIndex?: number;
}

interface ToolPairingResult {
	pairedResults: Map<number, Extract<MessagePart, { type: "tool_result" }>>;
	skippedResultIndices: Set<number>;
	taskGroups: Map<number, TaskGroup>;
	taskChildIndices: Set<number>;
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

	// Build task groups: find Task tool_use entries and collect children
	const taskGroups = new Map<number, TaskGroup>();
	const taskChildIndices = new Set<number>();

	// Map task tool_use ID → index for quick lookup
	const taskToolUseIdToIndex = new Map<string, number>();
	for (let i = 0; i < parts.length; i++) {
		const p = parts[i];
		if (p.type === "tool_use" && (p.tool === "Task" || p.tool === "Agent")) {
			const input = p.input as Record<string, unknown>;
			const group: TaskGroup = {
				toolUseIndex: i,
				toolUseId: p.id,
				description:
					typeof input.description === "string" ? input.description : undefined,
				subagentType:
					typeof input.subagent_type === "string"
						? input.subagent_type
						: undefined,
				childIndices: [],
				statusParts: [],
				resultIndex: undefined,
				isCompleted: false,
				isBackground: input.run_in_background === true,
			};
			taskGroups.set(i, group);
			taskToolUseIdToIndex.set(p.id, i);
		}
	}

	if (taskGroups.size > 0) {
		// Collect child parts (those with parentToolUseId matching a task)
		for (let i = 0; i < parts.length; i++) {
			const p = parts[i];
			if (p.type === "task_status") {
				const taskIdx = taskToolUseIdToIndex.get(p.taskToolUseId);
				if (taskIdx !== undefined) {
					const group = taskGroups.get(taskIdx);
					if (group) {
						group.statusParts.push(p);
						if (
							p.status === "completed" ||
							p.status === "failed" ||
							p.status === "stopped"
						) {
							group.isCompleted = true;
							if (group.isBackground) {
								group.completionStatusIndex = i;
							}
						}
					}
					taskChildIndices.add(i);
				}
				continue;
			}

			if ("parentToolUseId" in p && p.parentToolUseId) {
				const taskIdx = taskToolUseIdToIndex.get(p.parentToolUseId);
				if (taskIdx !== undefined) {
					const group = taskGroups.get(taskIdx);
					if (group) group.childIndices.push(i);
					taskChildIndices.add(i);
				}
			}
		}

		// Link task tool_result to group
		for (const [idx, group] of taskGroups) {
			const paired = pairedResults.get(idx);
			if (paired) {
				let resultIdx: number | undefined;
				for (const si of skippedResultIndices) {
					if (parts[si] === paired) {
						resultIdx = si;
						break;
					}
				}
				if (resultIdx !== undefined) {
					group.resultIndex = resultIdx;
					taskChildIndices.add(resultIdx);
					if (!group.isBackground) {
						group.isCompleted = true;
					}
				}
			}
		}
	}

	return { pairedResults, skippedResultIndices, taskGroups, taskChildIndices };
}
