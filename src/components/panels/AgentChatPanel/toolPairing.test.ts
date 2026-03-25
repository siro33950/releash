import { describe, expect, it } from "vitest";
import type { MessagePart } from "@/types/session";
import { buildToolPairings } from "./toolPairing";

describe("buildToolPairings", () => {
	it("returns empty maps for no parts", () => {
		const result = buildToolPairings([]);
		expect(result.pairedResults.size).toBe(0);
		expect(result.skippedResultIndices.size).toBe(0);
	});

	it("returns empty maps when there are no tool_use parts", () => {
		const parts: MessagePart[] = [
			{ type: "text", content: "hello" },
			{ type: "text", content: "world" },
		];
		const result = buildToolPairings(parts);
		expect(result.pairedResults.size).toBe(0);
		expect(result.skippedResultIndices.size).toBe(0);
	});

	it("pairs tool_use with tool_result by ID", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "text", content: "interleaved" },
			{ type: "tool_result", content: "ok", isError: false, toolUseId: "t1" },
		];
		const result = buildToolPairings(parts);
		expect(result.pairedResults.size).toBe(1);
		expect(result.pairedResults.get(0)?.content).toBe("ok");
		expect(result.skippedResultIndices.has(2)).toBe(true);
	});

	it("pairs tool_use with adjacent tool_result as fallback", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "tool_result", content: "ok", isError: false },
		];
		const result = buildToolPairings(parts);
		expect(result.pairedResults.size).toBe(1);
		expect(result.pairedResults.get(0)?.content).toBe("ok");
		expect(result.skippedResultIndices.has(1)).toBe(true);
	});

	it("prefers ID-based pairing over adjacent", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "tool_result", content: "adjacent", isError: false },
			{
				type: "tool_result",
				content: "by-id",
				isError: false,
				toolUseId: "t1",
			},
		];
		const result = buildToolPairings(parts);
		expect(result.pairedResults.get(0)?.content).toBe("by-id");
		expect(result.skippedResultIndices.has(2)).toBe(true);
		expect(result.skippedResultIndices.has(1)).toBe(false);
	});

	it("handles multiple tool_use/tool_result pairs", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "tool_result", content: "r1", isError: false, toolUseId: "t1" },
			{ type: "tool_use", tool: "Write", input: {}, id: "t2" },
			{ type: "tool_result", content: "r2", isError: false, toolUseId: "t2" },
		];
		const result = buildToolPairings(parts);
		expect(result.pairedResults.size).toBe(2);
		expect(result.pairedResults.get(0)?.content).toBe("r1");
		expect(result.pairedResults.get(2)?.content).toBe("r2");
		expect(result.skippedResultIndices.size).toBe(2);
	});

	it("leaves unpaired tool_use without result", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "text", content: "no result yet" },
		];
		const result = buildToolPairings(parts);
		expect(result.pairedResults.size).toBe(0);
		expect(result.skippedResultIndices.size).toBe(0);
	});

	it("does not double-assign adjacent result already paired by ID", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "A", input: {}, id: "t1" },
			{ type: "tool_use", tool: "B", input: {}, id: "t2" },
			{
				type: "tool_result",
				content: "for-t2",
				isError: false,
				toolUseId: "t2",
			},
		];
		const result = buildToolPairings(parts);
		// t2 is paired by ID
		expect(result.pairedResults.get(1)?.content).toBe("for-t2");
		// t1 has no pair (adjacent is already taken by t2's ID match)
		expect(result.pairedResults.has(0)).toBe(false);
	});

	it("returns empty taskGroups when no Task tool_use", () => {
		const parts: MessagePart[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "tool_result", content: "ok", isError: false, toolUseId: "t1" },
		];
		const result = buildToolPairings(parts);
		expect(result.taskGroups.size).toBe(0);
		expect(result.taskChildIndices.size).toBe(0);
	});

	it("creates task group for Task tool_use", () => {
		const parts: MessagePart[] = [
			{
				type: "tool_use",
				tool: "Task",
				input: { description: "Search code", subagent_type: "Explore" },
				id: "task1",
			},
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/a.ts" },
				id: "sub1",
				parentToolUseId: "task1",
			},
			{
				type: "tool_result",
				content: "file contents",
				isError: false,
				toolUseId: "sub1",
				parentToolUseId: "task1",
			},
			{
				type: "task_status",
				taskToolUseId: "task1",
				status: "completed",
				summary: "Found 1 file",
			},
			{
				type: "tool_result",
				content: "task done",
				isError: false,
				toolUseId: "task1",
			},
		];
		const result = buildToolPairings(parts);
		expect(result.taskGroups.size).toBe(1);
		const group = result.taskGroups.get(0);
		expect(group).toBeDefined();
		expect(group?.toolUseId).toBe("task1");
		expect(group?.description).toBe("Search code");
		expect(group?.subagentType).toBe("Explore");
		expect(group?.isCompleted).toBe(true);
		expect(group?.childIndices).toContain(1);
		expect(group?.childIndices).toContain(2);
		expect(result.taskChildIndices.has(1)).toBe(true);
		expect(result.taskChildIndices.has(2)).toBe(true);
		expect(result.taskChildIndices.has(3)).toBe(true);
	});

	it("marks task group as completed when tool_result is paired", () => {
		const parts: MessagePart[] = [
			{
				type: "tool_use",
				tool: "Task",
				input: { description: "Search code", subagent_type: "Explore" },
				id: "task1",
			},
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/a.ts" },
				id: "sub1",
				parentToolUseId: "task1",
			},
			{
				type: "tool_result",
				content: "file contents",
				isError: false,
				toolUseId: "sub1",
				parentToolUseId: "task1",
			},
			{
				type: "tool_result",
				content: "task done",
				isError: false,
				toolUseId: "task1",
			},
		];
		const result = buildToolPairings(parts);
		const group = result.taskGroups.get(0);
		expect(group).toBeDefined();
		expect(group?.isCompleted).toBe(true);
		expect(group?.resultIndex).toBe(3);
		expect(result.taskChildIndices.has(3)).toBe(true);
	});

	it("creates task group for Agent tool_use (SDK v0.2.77+)", () => {
		const parts: MessagePart[] = [
			{
				type: "tool_use",
				tool: "Agent",
				input: { description: "Explore codebase", subagent_type: "Explore" },
				id: "agent1",
			},
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/a.ts" },
				id: "sub1",
				parentToolUseId: "agent1",
			},
			{
				type: "tool_result",
				content: "file contents",
				isError: false,
				toolUseId: "sub1",
				parentToolUseId: "agent1",
			},
			{
				type: "tool_result",
				content: "agent done",
				isError: false,
				toolUseId: "agent1",
			},
		];
		const result = buildToolPairings(parts);
		expect(result.taskGroups.size).toBe(1);
		const group = result.taskGroups.get(0);
		expect(group).toBeDefined();
		expect(group?.toolUseId).toBe("agent1");
		expect(group?.description).toBe("Explore codebase");
		expect(group?.subagentType).toBe("Explore");
		expect(group?.isCompleted).toBe(true);
		expect(group?.childIndices).toContain(1);
		expect(group?.childIndices).toContain(2);
	});

	it("marks task group as not completed when only started", () => {
		const parts: MessagePart[] = [
			{
				type: "tool_use",
				tool: "Task",
				input: { description: "Running" },
				id: "task1",
			},
			{
				type: "task_status",
				taskToolUseId: "task1",
				status: "started",
			},
			{
				type: "tool_use",
				tool: "Read",
				input: {},
				id: "sub1",
				parentToolUseId: "task1",
			},
		];
		const result = buildToolPairings(parts);
		const group = result.taskGroups.get(0);
		expect(group).toBeDefined();
		expect(group?.isCompleted).toBe(false);
		expect(group?.childIndices).toContain(2);
	});
});
