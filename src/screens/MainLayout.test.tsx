/**
 * spec issues-1023 Rule「利用者は中央エリアから Workflow 観測モードへ切り替えられる」
 *
 * MainLayout 近傍の統合テスト。中央 ViewToolbar の mode toggle を click すると、
 * 中央エリアの AgentChatPanel と入れ替わりに WorkflowView が表示されること、
 * および右パネル上半分は常に ReviewPanel 専用であることを担保する。
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
// ViewToolbar の mode 切替UIを直接レンダリングする軽量モック。
// 中央エリアの "Agent mode" / "Workflow mode" toggle を click したときに
// onModeChange が呼ばれることを再現する。
vi.mock("@/components/layout/ViewToolbar", () => ({
	ViewToolbar: ({
		rightSlot,
		mode,
		onModeChange,
	}: {
		rightSlot?: React.ReactNode;
		mode?: "agent" | "workflow";
		onModeChange?: (mode: "agent" | "workflow") => void;
	}) => (
		<div data-testid="view-toolbar-mock">
			{mode !== undefined && onModeChange !== undefined && (
				<>
					<button
						type="button"
						aria-label="Agent mode"
						aria-pressed={mode === "agent"}
						onClick={() => onModeChange("agent")}
					/>
					<button
						type="button"
						aria-label="Workflow mode"
						aria-pressed={mode === "workflow"}
						onClick={() => onModeChange("workflow")}
					/>
				</>
			)}
			{rightSlot}
		</div>
	),
}));

const { MainLayout } = await import("./MainLayout");
const { DEFAULT_SETTINGS } = await import("@/types/settings");
const defaultSettings = DEFAULT_SETTINGS;

describe("MainLayout center mode switch", () => {
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

	it("mounts WorkflowView with selectedRootPath when Workflow mode is selected", () => {
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

		const panel = screen.getByTestId("workflow-view-mock");
		expect(panel).toBeInTheDocument();
		expect(panel).toHaveAttribute("data-worktree-path", "/managed/wt");
		// AgentChat は中央から消え、Review は右パネルで残る
		expect(screen.queryByTestId("agent-chat-panel-mock")).toBeNull();
		expect(screen.getByTestId("review-panel-mock")).toBeInTheDocument();
	});
});
