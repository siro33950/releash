/**
 * Goal #1454: the center pane is always the generic NodeContentView. A new
 * session is a creation request which is resolved to an opaque Workspace node
 * id by the backend before it becomes a CenterSelection.
 */
import { render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";

Element.prototype.scrollIntoView = vi.fn();

const mocks = vi.hoisted(() => ({
	createNewWorkspaceSession: vi.fn(),
	invoke: vi.fn(),
	nodeContentViewProps: vi.fn(),
}));

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({
		children,
		id,
		minSize,
	}: {
		children: React.ReactNode;
		id?: string;
		minSize?: number | string;
	}) => (
		<div data-testid={id ? `panel-${id}` : undefined} data-min-size={minSize}>
			{children}
		</div>
	),
	Separator: () => <div />,
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
	open: vi.fn(),
	confirm: vi.fn(),
}));

vi.mock("@/hooks/useWorkspacePersistence", () => ({
	useWorkspacePersistence: () => ({
		internalStateMapRef: { current: new Map() },
		getInitialState: () => undefined,
		stateReady: true,
	}),
}));
vi.mock("@/hooks/useCurrentBranch", () => ({
	useCurrentBranch: () => ({ branch: "feature" }),
}));
vi.mock("@/hooks/useBaseBranch", () => ({
	useBaseBranch: () => ({
		baseBranch: "main",
		setBaseBranch: vi.fn(),
		localBranches: ["main", "feature"],
	}),
}));
vi.mock("@/contexts/AgentChatContext", () => ({
	AgentChatProvider: ({ children }: { children: React.ReactNode }) => children,
	useAgentChatContext: () => ({
		createNewWorkspaceSession: mocks.createNewWorkspaceSession,
	}),
}));
vi.mock("@/contexts/ReviewThreadHandoffContext", () => ({
	ReviewThreadHandoffProvider: ({ children }: { children: React.ReactNode }) =>
		children,
}));

vi.mock("@/screens/useWorktreeState", () => ({
	useWorktreeState: () => ({
		selectedDiffFile: null,
		setSelectedDiffFile: vi.fn(),
		gitError: null,
		dispatchGit: vi.fn(),
		showCreateBranch: false,
		newBranchName: "",
		dispatchUI: vi.fn(),
		gitActions: { executeCreateBranch: vi.fn() },
		isSettingsOpen: false,
		diffOnlyMode: false,
		setDiffOnlyMode: vi.fn(),
		reviewCollapsed: false,
		setReviewCollapsed: vi.fn(),
		rightBottomCollapsed: false,
		setRightBottomCollapsed: vi.fn(),
		registerDropZone: vi.fn(),
	}),
}));

vi.mock("@/components/panels/NodeContentView", () => ({
	NodeContentView: (props: unknown) => {
		mocks.nodeContentViewProps(props);
		return (
			<div data-testid="node-content-view-mock">
				<div data-testid="node-toolbar-mock" />
			</div>
		);
	},
}));
vi.mock("@/components/panels/ReviewPanel", () => ({
	ReviewPanel: () => <div data-testid="review-panel-mock" />,
}));
vi.mock("@/components/panels/RightSidebarBottom", () => ({
	RightSidebarBottom: () => <div data-testid="right-bottom-mock" />,
}));
vi.mock("@/components/panels/SettingsModal", () => ({
	SettingsModal: () => null,
}));
vi.mock("@/screens/WorktreeViewDialogs", () => ({
	GitErrorDialog: () => null,
	CreateBranchDialog: () => null,
}));
vi.mock("@/components/layout/BranchSelector", () => ({
	BranchSelector: () => <div data-testid="branch-selector-mock" />,
}));
vi.mock("@/components/layout/RightPanelHeader", () => ({
	RightPanelHeader: ({ leftSlot }: { leftSlot?: React.ReactNode }) => (
		<div data-testid="right-panel-header-mock">{leftSlot}</div>
	),
}));
vi.mock("@/components/layout/ViewToolbar", () => ({
	ViewToolbar: () => <div data-testid="empty-state-toolbar-mock" />,
}));

const { MainLayout } = await import("./MainLayout");
const { DEFAULT_SETTINGS } = await import("@/types/settings");

function mainLayoutElement(
	props: Partial<React.ComponentProps<typeof MainLayout>> = {},
) {
	return (
		<StrictMode>
			<TooltipProvider>
				<MainLayout
					selectedRootPath="/managed/wt"
					settings={DEFAULT_SETTINGS}
					onSettingsSave={vi.fn()}
					leftNav={<div />}
					{...props}
				/>
			</TooltipProvider>
		</StrictMode>
	);
}

function renderMainLayout(
	props: Partial<React.ComponentProps<typeof MainLayout>> = {},
) {
	return render(mainLayoutElement(props));
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe("MainLayout node-centered workspace", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mocks.invoke.mockResolvedValue(null);
		mocks.createNewWorkspaceSession.mockResolvedValue("session-default");
	});

	it("always renders the toolbar-backed NodeContentView in the center", () => {
		renderMainLayout();

		expect(screen.getByTestId("node-content-view-mock")).toBeInTheDocument();
		expect(screen.getByTestId("node-toolbar-mock")).toBeInTheDocument();
		expect(screen.getByTestId("review-panel-mock")).toBeInTheDocument();
		expect(mocks.nodeContentViewProps).toHaveBeenLastCalledWith(
			expect.objectContaining({
				worktreePath: "/managed/wt",
				nodeId: null,
			}),
		);
	});

	it("passes an opaque node selection to the generic center view", () => {
		renderMainLayout({
			centerSelection: {
				kind: "node",
				worktreePath: "/managed/wt",
				nodeId: "opaque:not-an-execution-coordinate",
			},
		});

		expect(mocks.nodeContentViewProps).toHaveBeenLastCalledWith(
			expect.objectContaining({
				worktreePath: "/managed/wt",
				nodeId: "opaque:not-an-execution-coordinate",
			}),
		);
		expect(screen.getByTestId("panel-center")).toHaveAttribute(
			"data-min-size",
			"30%",
		);
	});

	it("does not leak another worktree's selection into the current view", () => {
		renderMainLayout({
			centerSelection: {
				kind: "node",
				worktreePath: "/managed/other",
				nodeId: "node-other",
			},
		});

		expect(mocks.nodeContentViewProps).toHaveBeenLastCalledWith(
			expect.objectContaining({ nodeId: null }),
		);
	});

	it("resolves NewSession through the backend before selecting its Node", async () => {
		mocks.createNewWorkspaceSession.mockResolvedValue("session-new");
		mocks.invoke.mockResolvedValue("node-opaque-new");
		const onNewSessionCreated = vi.fn();
		const refreshListener = vi.fn();
		window.addEventListener("workspace-tree-refresh", refreshListener);
		const request = {
			worktreePath: "/managed/wt",
			requestId: "request-7",
			attempt: 1,
		};

		renderMainLayout({
			newSessionCreationRequest: request,
			onNewSessionCreated,
		});

		await waitFor(() => {
			expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1);
			expect(mocks.createNewWorkspaceSession).toHaveBeenCalledWith("request-7");
			expect(mocks.invoke).toHaveBeenCalledWith(
				"get_workspace_session_node_id",
				{
					worktreePath: "/managed/wt",
					sessionId: "session-new",
				},
			);
			expect(onNewSessionCreated).toHaveBeenCalledWith(request, {
				kind: "node",
				worktreePath: "/managed/wt",
				nodeId: "node-opaque-new",
			});
			expect(refreshListener).toHaveBeenCalledTimes(1);
		});

		window.removeEventListener("workspace-tree-refresh", refreshListener);
	});

	it("keeps one creation task across Worktree unmount and remount", async () => {
		const pending = deferred<string>();
		mocks.createNewWorkspaceSession.mockReturnValue(pending.promise);
		mocks.invoke.mockResolvedValue("node-a");
		const onNewSessionCreated = vi.fn();
		const request = {
			worktreePath: "/managed/wt",
			requestId: "request-a",
			attempt: 1,
		};
		const view = renderMainLayout({
			selectedRootPath: "/managed/wt",
			newSessionCreationRequest: request,
			onNewSessionCreated,
		});
		await waitFor(() =>
			expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1),
		);

		view.rerender(
			mainLayoutElement({
				selectedRootPath: "/managed/other",
				newSessionCreationRequest: null,
				onNewSessionCreated,
			}),
		);
		view.rerender(
			mainLayoutElement({
				selectedRootPath: "/managed/wt",
				newSessionCreationRequest: request,
				onNewSessionCreated,
			}),
		);
		expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1);

		pending.resolve("session-a");
		await waitFor(() => {
			expect(onNewSessionCreated).toHaveBeenCalledOnce();
			expect(onNewSessionCreated).toHaveBeenCalledWith(request, {
				kind: "node",
				worktreePath: "/managed/wt",
				nodeId: "node-a",
			});
		});
		expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1);
	});

	it("prunes a settled creation task after its request is acknowledged", async () => {
		mocks.createNewWorkspaceSession.mockResolvedValue("session-pruned");
		mocks.invoke.mockResolvedValue("node-pruned");
		const onNewSessionCreated = vi.fn();
		const request = {
			worktreePath: "/managed/wt",
			requestId: "request-pruned",
			attempt: 1,
		};
		const view = renderMainLayout({
			newSessionCreationRequest: request,
			onNewSessionCreated,
		});
		await waitFor(() => expect(onNewSessionCreated).toHaveBeenCalledOnce());
		expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1);

		view.rerender(
			mainLayoutElement({
				newSessionCreationRequest: null,
				onNewSessionCreated,
			}),
		);
		view.rerender(
			mainLayoutElement({
				newSessionCreationRequest: request,
				onNewSessionCreated,
			}),
		);

		await waitFor(() =>
			expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(2),
		);
	});

	it("keeps a pending task deduplicated until a failed attempt settles", async () => {
		const pending = deferred<string>();
		mocks.createNewWorkspaceSession.mockReturnValue(pending.promise);
		const onNewSessionCreationFailed = vi.fn();
		const request = {
			worktreePath: "/managed/wt",
			requestId: "request-failed",
			attempt: 1,
		};
		const view = renderMainLayout({
			newSessionCreationRequest: request,
			onNewSessionCreationFailed,
		});
		await waitFor(() =>
			expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1),
		);

		view.rerender(
			mainLayoutElement({
				selectedRootPath: "/managed/other",
				newSessionCreationRequest: null,
				onNewSessionCreationFailed,
			}),
		);
		view.rerender(
			mainLayoutElement({
				newSessionCreationRequest: request,
				onNewSessionCreationFailed,
			}),
		);
		expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1);

		pending.reject(new Error("offline"));
		await waitFor(() =>
			expect(onNewSessionCreationFailed).toHaveBeenCalledWith(
				request,
				"offline",
			),
		);
		expect(mocks.createNewWorkspaceSession).toHaveBeenCalledTimes(1);
	});

	it("keeps BranchSelector in the right panel header", () => {
		renderMainLayout();

		expect(screen.getByTestId("right-panel-header-mock")).toContainElement(
			screen.getByTestId("branch-selector-mock"),
		);
	});
});
