import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { WorkspaceNodeDetailDto as WorkspaceNodeDetail } from "@/generated/client_types";
import { invokeClient, subscribeState } from "@/lib/client";
import {
	approveWorkspaceNode,
	resumeWorkspaceSessionNode,
	retryWorkspaceNode,
	useWorkspaceNodeDetail,
} from "./useWorkspaceNodeDetail";

vi.mock("@/lib/client", () => ({
	invokeClient: vi.fn(),
	subscribeState: vi.fn(),
}));
beforeEach(() => vi.clearAllMocks());
const detail: WorkspaceNodeDetail = {
	id: "node",
	title: "Node",
	processPresence: "confirmed_absent",
	status: "completed",
	statusClassification: "idle",
	submitReceived: false,
	stopReceived: false,
	hasArtifact: false,
	capabilities: {
		canApprove: false,
		canRetry: false,
		canResumeSession: false,
		canRename: false,
	},
	updatedAt: 1,
	content: { kind: "session", sessionId: "session" },
};
it("詳細と削除は購読から反映され対象変更で前の購読を解除する", () => {
	let receive!: (value: WorkspaceNodeDetail | null) => void;
	const stop = vi.fn();
	vi.mocked(subscribeState).mockImplementation((_target, next) => {
		receive = next;
		return stop;
	});
	const { result, rerender, unmount } = renderHook(
		({ nodeId }) => useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId }),
		{ initialProps: { nodeId: "node" } },
	);
	expect(result.current.loading).toBe(true);
	act(() => receive(detail));
	expect(result.current.detail).toEqual(detail);
	act(() => receive(null));
	expect(result.current.missingNodeId).toBe("node");
	rerender({ nodeId: "other" });
	expect(result.current.detail).toBeNull();
	expect(result.current.loading).toBe(true);
	expect(stop).toHaveBeenCalledOnce();
	unmount();
	expect(stop).toHaveBeenCalledTimes(2);
	expect(invokeClient).not.toHaveBeenCalled();
});
it.each([
	[approveWorkspaceNode, "approve_workspace_node"],
	[retryWorkspaceNode, "retry_workspace_node"],
	[resumeWorkspaceSessionNode, "resume_workspace_session_node"],
] as const)(
	"%sは操作だけを要求し詳細を取り直さない",
	async (operation, command) => {
		vi.mocked(invokeClient).mockResolvedValueOnce(undefined);
		expect(
			await operation({ worktreePath: "/repo", nodeId: "node" }),
		).toBeUndefined();
		expect(invokeClient).toHaveBeenCalledExactlyOnceWith(command, {
			worktreePath: "/repo",
			nodeId: "node",
		});
	},
);
it("対象が無いときは購読しない", () => {
	const { result } = renderHook(() =>
		useWorkspaceNodeDetail({ worktreePath: null, nodeId: null }),
	);
	expect(result.current.loading).toBe(false);
	expect(subscribeState).not.toHaveBeenCalled();
});
