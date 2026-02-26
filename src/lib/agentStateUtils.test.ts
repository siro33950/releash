import { describe, expect, it } from "vitest";
import type { AgentStateSync } from "@/types/protocol";
import {
	agentStateKey,
	aggregateAgentState,
	aggregateFromEntries,
	highestPriorityState,
} from "./agentStateUtils";

function makeSync(
	worktreePath: string,
	state: AgentStateSync["state"],
	ptyId?: string,
): AgentStateSync {
	return {
		worktree_path: worktreePath,
		state,
		exit_code: null,
		timestamp: Date.now(),
		session_id: null,
		pty_id: ptyId ?? null,
	};
}

describe("aggregateAgentState", () => {
	it("returns undefined for empty map", () => {
		const map = new Map<string, AgentStateSync>();
		expect(aggregateAgentState(map, "/repo")).toBeUndefined();
	});

	it("returns the state of a single matching entry", () => {
		const map = new Map<string, AgentStateSync>();
		map.set("/repo", makeSync("/repo", "done"));
		expect(aggregateAgentState(map, "/repo")).toBe("done");
	});

	it("returns highest priority state across multiple PTYs", () => {
		const map = new Map<string, AgentStateSync>();
		map.set("/repo::1", makeSync("/repo", "done", "1"));
		map.set("/repo::2", makeSync("/repo", "running", "2"));
		map.set("/repo::3", makeSync("/repo", "waiting", "3"));
		expect(aggregateAgentState(map, "/repo")).toBe("waiting");
	});

	it("ignores entries with different worktree_path", () => {
		const map = new Map<string, AgentStateSync>();
		map.set("/other::1", makeSync("/other", "running", "1"));
		map.set("/repo::1", makeSync("/repo", "done", "1"));
		expect(aggregateAgentState(map, "/repo")).toBe("done");
	});

	it("error beats waiting and done", () => {
		const map = new Map<string, AgentStateSync>();
		map.set("/repo::1", makeSync("/repo", "error", "1"));
		map.set("/repo::2", makeSync("/repo", "waiting", "2"));
		map.set("/repo::3", makeSync("/repo", "done", "3"));
		expect(aggregateAgentState(map, "/repo")).toBe("error");
	});
});

describe("highestPriorityState", () => {
	it("returns undefined for empty array", () => {
		expect(highestPriorityState([])).toBeUndefined();
	});

	it("returns the single state", () => {
		expect(highestPriorityState(["running"])).toBe("running");
	});

	it("error beats all others", () => {
		expect(highestPriorityState(["done", "running", "waiting", "error"])).toBe(
			"error",
		);
	});

	it("waiting beats running and done", () => {
		expect(highestPriorityState(["done", "running", "waiting"])).toBe(
			"waiting",
		);
	});
});

describe("aggregateFromEntries", () => {
	it("returns undefined for empty array", () => {
		expect(aggregateFromEntries([])).toBeUndefined();
	});

	it("returns highest priority from entries", () => {
		const entries = [
			makeSync("/repo", "done", "1"),
			makeSync("/repo", "error", "2"),
			makeSync("/repo", "running", "3"),
		];
		expect(aggregateFromEntries(entries)).toBe("error");
	});
});

describe("agentStateKey", () => {
	it("returns worktreePath::ptyId when ptyId is present", () => {
		expect(agentStateKey("/repo", "42")).toBe("/repo::42");
	});

	it("returns worktreePath only when ptyId is null", () => {
		expect(agentStateKey("/repo", null)).toBe("/repo");
	});

	it("returns worktreePath only when ptyId is undefined", () => {
		expect(agentStateKey("/repo")).toBe("/repo");
	});

	it("returns worktreePath only when ptyId is empty string", () => {
		expect(agentStateKey("/repo", "")).toBe("/repo");
	});
});
