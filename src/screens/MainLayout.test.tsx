/**
 * spec issues-1023 Rule「利用者は右パネルから Workflow 観測モードへ切り替えられる」
 *
 * MainLayout 近傍の統合テスト。右パネルの mode toggle を clic すると、選択中の
 * worktree (selectedRootPath) を prop に持つ WorkflowSidebarPanel が表示されることを担保する。
 */
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";

// jsdom does not implement scrollIntoView / ResizeObserver
Element.prototype.scrollIntoView = vi.fn();

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
	AgentChatPanel: () => <div data-testid="agent-chat-panel-mock" />,
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
vi.mock("@/components/panels/WorkflowSidebarPanel", () => ({
	WorkflowSidebarPanel: ({ worktreePath }: { worktreePath: string }) => (
		<div
			data-testid="workflow-sidebar-panel-mock"
			data-worktree-path={worktreePath}
		/>
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
	ViewToolbar: ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	),
}));

const { MainLayout } = await import("./MainLayout");
const { DEFAULT_SETTINGS } = await import("@/types/settings");
const defaultSettings = DEFAULT_SETTINGS;

describe("MainLayout right panel mode switch", () => {
	it("renders ReviewPanel by default and hides WorkflowSidebarPanel", () => {
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
		expect(screen.getByTestId("review-panel-mock")).toBeInTheDocument();
		expect(screen.queryByTestId("workflow-sidebar-panel-mock")).toBeNull();
	});

	it("mounts WorkflowSidebarPanel with selectedRootPath when Workflow mode is selected", () => {
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

		fireEvent.click(screen.getByLabelText("Workflow mode"));

		const panel = screen.getByTestId("workflow-sidebar-panel-mock");
		expect(panel).toBeInTheDocument();
		expect(panel).toHaveAttribute("data-worktree-path", "/managed/wt");
		expect(screen.queryByTestId("review-panel-mock")).toBeNull();
	});
});
