/**
 * spec issues-1220 Rule「中央表示は CenterSelection から導出される」
 *
 * MainLayout 近傍の統合テスト。Workspace tree から workflowRun selection request が
 * 渡ると、中央エリアの AgentChatPanel と入れ替わりに WorkflowView が表示されることを
 * 担保する。
 */
import { act, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";

// jsdom does not implement scrollIntoView / ResizeObserver
Element.prototype.scrollIntoView = vi.fn();

const agentChatPanelProps = vi.hoisted(() => ({
	current: null as {
		onNewSessionCreated?: (sessionId: string) => void;
	} | null,
}));

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue([]),
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
vi.mock("@/hooks/useWorkflowState", () => ({
	useWorkflowState: () => ({ workflowState: null }),
}));
vi.mock("@/contexts/AgentChatContext", () => ({
	AgentChatProvider: ({ children }: { children: React.ReactNode }) => children,
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
	}),
}));

vi.mock("@/components/panels/AgentChatPanel", () => ({
	AgentChatPanel: (props: {
		onNewSessionCreated?: (sessionId: string) => void;
	}) => {
		agentChatPanelProps.current = props;
		return <div data-testid="agent-chat-panel-mock" />;
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
vi.mock("@/components/panels/WorkflowView", () => ({
	WorkflowView: ({ worktreePath }: { worktreePath: string }) => (
		<div data-testid="workflow-view-mock" data-worktree-path={worktreePath} />
	),
}));
vi.mock("@/screens/WorktreeViewDialogs", () => ({
	GitErrorDialog: () => null,
	CreateBranchDialog: () => null,
}));
vi.mock("@/components/layout/BranchSelector", () => ({
	BranchSelector: () => <div data-testid="branch-selector-mock" />,
}));
vi.mock("@/components/layout/ViewToolbar", () => ({
	ViewToolbar: ({ rightSlot }: { rightSlot?: React.ReactNode }) => (
		<div data-testid="view-toolbar-mock">{rightSlot}</div>
	),
}));

const { MainLayout } = await import("./MainLayout");
const { DEFAULT_SETTINGS } = await import("@/types/settings");
const defaultSettings = DEFAULT_SETTINGS;

describe("MainLayout center selection", () => {
	it("renders AgentChatPanel and ReviewPanel by default", () => {
		render(
			<TooltipProvider>
				<MainLayout
					selectedRootPath="/managed/wt"
					settings={defaultSettings}
					onSettingsSave={vi.fn()}
					leftNav={<div />}
				/>
			</TooltipProvider>,
		);
		expect(screen.getByTestId("agent-chat-panel-mock")).toBeInTheDocument();
		expect(screen.getByTestId("review-panel-mock")).toBeInTheDocument();
		expect(screen.queryByTestId("workflow-view-mock")).toBeNull();
	});

	it("mounts WorkflowView with selectedRootPath when workflowRun selection is requested", async () => {
		render(
			<TooltipProvider>
				<MainLayout
					selectedRootPath="/managed/wt"
					settings={defaultSettings}
					onSettingsSave={vi.fn()}
					leftNav={<div />}
					centerSelectionRequest={{
						kind: "workflowRun",
						worktreePath: "/managed/wt",
						runId: "run-1",
						requestId: 1,
					}}
				/>
			</TooltipProvider>,
		);

		const panel = await screen.findByTestId("workflow-view-mock");
		expect(panel).toBeInTheDocument();
		expect(panel).toHaveAttribute("data-worktree-path", "/managed/wt");
		// AgentChat は中央から消え、Review は右パネルで残る
		expect(screen.queryByTestId("agent-chat-panel-mock")).toBeNull();
		await waitFor(() => {
			expect(screen.getByTestId("review-panel-mock")).toBeInTheDocument();
		});
	});

	it("resolves newAgentSession selection to the created agent session", () => {
		agentChatPanelProps.current = null;
		const onCenterSelectionResolved = vi.fn();
		render(
			<TooltipProvider>
				<MainLayout
					selectedRootPath="/managed/wt"
					settings={defaultSettings}
					onSettingsSave={vi.fn()}
					leftNav={<div />}
					centerSelectionRequest={{
						kind: "newAgentSession",
						worktreePath: "/managed/wt",
						requestId: 1,
					}}
					onCenterSelectionResolved={onCenterSelectionResolved}
				/>
			</TooltipProvider>,
		);

		act(() => {
			agentChatPanelProps.current?.onNewSessionCreated?.("new-s");
		});

		expect(onCenterSelectionResolved).toHaveBeenCalledWith({
			kind: "agentSession",
			worktreePath: "/managed/wt",
			sessionId: "new-s",
		});
	});
});
