import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

interface TestRequest {
	requestId: string;
	worktreePath: string;
	attempt: number;
}

const mocks = vi.hoisted(() => ({
	openWorktreeTab: vi.fn(),
	selectedWorktreeId: "wt-a",
	preferredNodeId: "preferred-a" as string | null,
	lastRequests: new Map<string, TestRequest>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn((command: string) => {
		if (command === "get_application_startup_outcome") {
			return Promise.resolve({ type: "ready" });
		}
		if (command === "get_cwd" || command === "get_main_repo_path") {
			return Promise.resolve("/repo");
		}
		if (command === "list_worktrees") return Promise.resolve([]);
		return Promise.resolve(null);
	}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/hooks/useSettings", () => ({
	useSettings: () => ({
		settings: { autoUpdate: false, theme: "dark" },
		updateSettings: vi.fn(),
		updateTheme: vi.fn(),
	}),
}));
vi.mock("@/hooks/useUpdateChecker", () => ({
	useUpdateChecker: () => null,
}));
vi.mock("@/hooks/useWorkspaceNavigation", () => ({
	useWorkspaceNavigation: () => ({
		worktrees: [
			{ id: "wt-a", rootPath: "/wt-a" },
			{ id: "wt-b", rootPath: "/wt-b" },
		],
		selectedWorktreeId: mocks.selectedWorktreeId,
		openWorktreeTab: mocks.openWorktreeTab,
	}),
}));
vi.mock("@/hooks/useRepoList", () => ({
	useRepoList: () => ({
		repoPaths: ["/repo"],
		addRepo: vi.fn(),
		removeRepo: vi.fn(),
		initFromCwd: vi.fn(),
	}),
}));
vi.mock("@/hooks/useMenuEvents", () => ({ useMenuEvents: vi.fn() }));
vi.mock("@/components/UpdateDialog", () => ({ UpdateDialog: () => null }));
vi.mock("@/components/panels/SettingsModal", () => ({
	SettingsModal: () => null,
}));
vi.mock("@/components/workspace/WorkspaceList", () => ({
	WorkspaceList: ({
		autoSelectPreferredNode,
		newSessionCreationStatusByWorktree,
		onCreateSession,
		onSelectWorktree,
	}: {
		autoSelectPreferredNode?: boolean;
		newSessionCreationStatusByWorktree?: Record<
			string,
			{ pending: boolean; error: string | null }
		>;
		onCreateSession: (worktreePath: string) => void;
		onSelectWorktree: (
			worktreePath: string,
			branch?: string,
			repo?: string,
			selection?: {
				kind: "node";
				worktreePath: string;
				nodeId: string;
			},
		) => void;
	}) => (
		<div>
			<div data-testid="auto-select">
				{autoSelectPreferredNode ? "awaiting" : "settled"}
			</div>
			<div data-testid="creation-error-a">
				{newSessionCreationStatusByWorktree?.["/wt-a"]?.error ?? "none"}
			</div>
			<button type="button" onClick={() => onCreateSession("/wt-a")}>
				Create A
			</button>
			<button type="button" onClick={() => onCreateSession("/wt-b")}>
				Create B
			</button>
			<button
				type="button"
				onClick={() =>
					onSelectWorktree("/wt-a", undefined, undefined, {
						kind: "node",
						worktreePath: "/wt-a",
						nodeId: "node-a",
					})
				}
			>
				Select A
			</button>
			<button
				type="button"
				onClick={() =>
					autoSelectPreferredNode &&
					mocks.preferredNodeId &&
					onSelectWorktree("/wt-a", undefined, undefined, {
						kind: "node",
						worktreePath: "/wt-a",
						nodeId: mocks.preferredNodeId,
					})
				}
			>
				Apply preferred A
			</button>
		</div>
	),
}));
vi.mock("@/screens/MainLayout", () => ({
	MainLayout: ({
		selectedRootPath,
		leftNav,
		centerSelection,
		newSessionCreationRequest,
		onNewSessionCreated,
		onNewSessionCreationFailed,
		onCenterNodeMissing,
	}: {
		selectedRootPath: string | null;
		leftNav: React.ReactNode;
		centerSelection?: {
			kind: "node";
			worktreePath: string;
			nodeId: string;
		} | null;
		newSessionCreationRequest?: TestRequest | null;
		onNewSessionCreated?: (
			request: TestRequest,
			selection: {
				kind: "node";
				worktreePath: string;
				nodeId: string;
			},
		) => void;
		onNewSessionCreationFailed?: (request: TestRequest, error: string) => void;
		onCenterNodeMissing?: (worktreePath: string, nodeId: string) => void;
	}) => {
		if (newSessionCreationRequest) {
			mocks.lastRequests.set(
				newSessionCreationRequest.worktreePath,
				newSessionCreationRequest,
			);
		}
		return (
			<div>
				{leftNav}
				<div data-testid="active-worktree">{selectedRootPath ?? "none"}</div>
				<div data-testid="request-id">
					{newSessionCreationRequest?.requestId ?? "none"}
				</div>
				<div data-testid="request-attempt">
					{newSessionCreationRequest?.attempt ?? "none"}
				</div>
				<div data-testid="center-node">{centerSelection?.nodeId ?? "none"}</div>
				<button
					type="button"
					onClick={() => {
						if (!newSessionCreationRequest) return;
						onNewSessionCreated?.(newSessionCreationRequest, {
							kind: "node",
							worktreePath: newSessionCreationRequest.worktreePath,
							nodeId: `created-${newSessionCreationRequest.worktreePath}`,
						});
					}}
				>
					Resolve current
				</button>
				<button
					type="button"
					onClick={() => {
						if (!newSessionCreationRequest) return;
						onNewSessionCreationFailed?.(newSessionCreationRequest, "offline");
					}}
				>
					Fail current
				</button>
				<button
					type="button"
					onClick={() => {
						const request = mocks.lastRequests.get("/wt-a");
						if (!request) return;
						onNewSessionCreated?.(request, {
							kind: "node",
							worktreePath: "/wt-a",
							nodeId: "created-/wt-a",
						});
					}}
				>
					Resolve saved A
				</button>
				<button
					type="button"
					onClick={() => {
						if (!centerSelection) return;
						onCenterNodeMissing?.(
							centerSelection.worktreePath,
							centerSelection.nodeId,
						);
					}}
				>
					Invalidate center
				</button>
			</div>
		);
	},
}));

const { default: App } = await import("./App");

beforeEach(() => {
	vi.clearAllMocks();
	mocks.selectedWorktreeId = "wt-a";
	mocks.preferredNodeId = "preferred-a";
	mocks.lastRequests.clear();
});

describe("App Workspace selection lifecycle", () => {
	it("does not consume initial selection until preferred Node exists", async () => {
		render(<App />);
		expect(await screen.findByTestId("auto-select")).toHaveTextContent(
			"awaiting",
		);

		fireEvent.click(screen.getByRole("button", { name: "Apply preferred A" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent("preferred-a");
		expect(screen.getByTestId("auto-select")).toHaveTextContent("settled");
	});

	it("falls back to the new preferred Node after authoritative removal", async () => {
		render(<App />);
		await screen.findByRole("button", { name: "Select A" });
		fireEvent.click(screen.getByRole("button", { name: "Select A" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent("node-a");

		mocks.preferredNodeId = "replacement-a";
		fireEvent.click(screen.getByRole("button", { name: "Invalidate center" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent("none");
		expect(screen.getByTestId("auto-select")).toHaveTextContent("awaiting");
		fireEvent.click(screen.getByRole("button", { name: "Apply preferred A" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent(
			"replacement-a",
		);
		expect(screen.getByTestId("auto-select")).toHaveTextContent("settled");
	});

	it("stays unselected when authoritative removal has no preferred Node", async () => {
		render(<App />);
		await screen.findByRole("button", { name: "Select A" });
		fireEvent.click(screen.getByRole("button", { name: "Select A" }));
		mocks.preferredNodeId = null;

		fireEvent.click(screen.getByRole("button", { name: "Invalidate center" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent("none");
		expect(screen.getByTestId("auto-select")).toHaveTextContent("awaiting");
		fireEvent.click(screen.getByRole("button", { name: "Apply preferred A" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent("none");
	});
});

describe("App NewSession creation requests", () => {
	it("deduplicates pending clicks and retries a failure with the same request id", async () => {
		render(<App />);

		await screen.findByRole("button", { name: "Create A" });
		fireEvent.click(screen.getByRole("button", { name: "Create A" }));
		const requestId = screen.getByTestId("request-id").textContent;
		expect(requestId).not.toBe("none");
		expect(screen.getByTestId("request-attempt")).toHaveTextContent("1");
		fireEvent.click(screen.getByRole("button", { name: "Create A" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent(requestId ?? "");
		expect(screen.getByTestId("request-attempt")).toHaveTextContent("1");

		fireEvent.click(screen.getByRole("button", { name: "Fail current" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent("none");
		expect(screen.getByTestId("creation-error-a")).toHaveTextContent(
			"Session creation failed: offline",
		);

		fireEvent.click(screen.getByRole("button", { name: "Create A" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent(requestId ?? "");
		expect(screen.getByTestId("request-attempt")).toHaveTextContent("2");
		fireEvent.click(screen.getByRole("button", { name: "Resolve current" }));
		expect(screen.getByTestId("request-id")).toHaveTextContent("none");
		expect(screen.getByTestId("center-node")).toHaveTextContent(
			"created-/wt-a",
		);
	});

	it("records an inactive Worktree completion without changing the active center", async () => {
		const view = render(<App />);
		await screen.findByRole("button", { name: "Create A" });
		fireEvent.click(screen.getByRole("button", { name: "Create A" }));

		mocks.selectedWorktreeId = "wt-b";
		view.rerender(<App />);
		expect(screen.getByTestId("active-worktree")).toHaveTextContent("/wt-b");
		expect(screen.getByTestId("center-node")).toHaveTextContent("none");
		fireEvent.click(screen.getByRole("button", { name: "Resolve saved A" }));
		expect(screen.getByTestId("center-node")).toHaveTextContent("none");

		mocks.selectedWorktreeId = "wt-a";
		view.rerender(<App />);
		expect(screen.getByTestId("center-node")).toHaveTextContent(
			"created-/wt-a",
		);
		expect(screen.getByTestId("request-id")).toHaveTextContent("none");
	});
});
