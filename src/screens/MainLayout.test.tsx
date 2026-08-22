import { fireEvent, render, screen, within } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";

Element.prototype.scrollIntoView = vi.fn();

const mocks = vi.hoisted(() => ({
	nodeContentViewProps: vi.fn(),
}));

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({
		children,
		id,
		minSize,
		onResize,
	}: {
		children: React.ReactNode;
		id?: string;
		minSize?: number | string;
		onResize?: (size: { asPercentage: number; inPixels: number }) => void;
	}) => (
		<div data-testid={id ? `panel-${id}` : undefined} data-min-size={minSize}>
			{id && onResize ? (
				<>
					<button
						type="button"
						data-testid={`panel-${id}-collapse-trigger`}
						onClick={() => onResize({ asPercentage: 0, inPixels: 0 })}
					/>
					<button
						type="button"
						data-testid={`panel-${id}-expand-trigger`}
						onClick={() => onResize({ asPercentage: 50, inPixels: 400 })}
					/>
				</>
			) : null}
			{children}
		</div>
	),
	Separator: () => <div />,
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
	RightPanelHeader: ({
		leftSlot,
		panels,
	}: {
		leftSlot?: React.ReactNode;
		panels?: { id: string; visible: boolean }[];
	}) => (
		<div
			data-testid="right-panel-header-mock"
			data-right-visible={panels?.find((p) => p.id === "right")?.visible}
		>
			{leftSlot}
		</div>
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

describe("MainLayout node-centered workspace", () => {
	beforeEach(() => {
		vi.clearAllMocks();
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

	it("Node選択の初期Session attachmentを中央表示へ渡して消費を通知する", () => {
		const onCenterSessionAttachmentConsumed = vi.fn();
		const initialSessionAttachment = {
			agentSessionId: "agent-session-1",
			workspaceIdentity: "/managed/wt",
			worktreePath: "/managed/wt",
			provider: "codex",
		};
		renderMainLayout({
			centerSelectionByWorktree: {
				"/managed/wt": {
					kind: "node",
					worktreePath: "/managed/wt",
					nodeId: "session-node-1",
					initialSessionAttachment,
				},
			},
			onCenterSessionAttachmentConsumed,
		});

		const calls = mocks.nodeContentViewProps.mock.calls;
		const props = calls[calls.length - 1]?.[0] as {
			initialSessionAttachment?: unknown;
			onInitialSessionConsumed?: (agentSessionId: string) => void;
		};
		expect(props.initialSessionAttachment).toEqual(initialSessionAttachment);
		props.onInitialSessionConsumed?.("agent-session-1");
		expect(onCenterSessionAttachmentConsumed).toHaveBeenCalledWith(
			"/managed/wt",
			"session-node-1",
			"agent-session-1",
		);
	});

	it("app-wide warningをWorkspace外のmain-area内で高さを確保して表示する", () => {
		renderMainLayout({
			topBanner: <div role="alert">Provider warning</div>,
		});

		const warning = screen.getByRole("alert");
		const leftNav = screen.getByTestId("panel-left-nav");
		const mainArea = screen.getByTestId("panel-main-area");
		const bannerRegion = screen.getByTestId("main-layout-banner-region");
		const contentRegion = screen.getByTestId("main-layout-content-region");

		expect(leftNav).not.toContainElement(warning);
		expect(mainArea).toContainElement(warning);
		expect(bannerRegion).toHaveClass("shrink-0");
		expect(contentRegion).toHaveClass("min-h-0", "flex-1");
	});

	it("passes an opaque node selection to the generic center view", () => {
		renderMainLayout({
			centerSelectionByWorktree: {
				"/managed/wt": {
					kind: "node",
					worktreePath: "/managed/wt",
					nodeId: "opaque:not-an-execution-coordinate",
				},
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

	it("Session作成中だけ一時的なlaunching表示を使う", () => {
		renderMainLayout({
			centerSelectionByWorktree: {
				"/managed/wt": {
					kind: "agent_session_launching",
					worktreePath: "/managed/wt",
					provider: "codex",
					launchToken: "launch-1",
				},
			},
		});

		expect(screen.getByTestId("agent-session-launching")).toBeInTheDocument();
		expect(screen.queryByTestId("node-content-view-mock")).toBeNull();
	});

	it("does not leak another worktree's selection into the current view", () => {
		renderMainLayout({
			centerSelectionByWorktree: {
				"/managed/other": {
					kind: "node",
					worktreePath: "/managed/other",
					nodeId: "node-other",
				},
			},
		});

		expect(mocks.nodeContentViewProps).toHaveBeenLastCalledWith(
			expect.objectContaining({ nodeId: null }),
		);
	});

	it("keeps BranchSelector in the right panel header", () => {
		renderMainLayout();

		expect(screen.getByTestId("right-panel-header-mock")).toContainElement(
			screen.getByTestId("branch-selector-mock"),
		);
	});
});

describe("MainLayout keep-mounted panes", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	function rightSlotHasToggleFor(worktreePath: string): boolean {
		const calls = mocks.nodeContentViewProps.mock.calls
			.map(
				(call) =>
					call[0] as { worktreePath: string; rightSlot?: React.ReactNode },
			)
			.filter((p) => p.worktreePath === worktreePath);
		const props = calls[calls.length - 1];
		const slotView = render(
			<TooltipProvider>{props?.rightSlot}</TooltipProvider>,
		);
		const hasToggle =
			within(slotView.container).queryByLabelText("Toggle Right Sidebar") !==
			null;
		slotView.unmount();
		return hasToggle;
	}

	it("keeps the previous pane mounted and hidden after switching worktrees", () => {
		const view = renderMainLayout({ selectedRootPath: "/managed/a" });
		view.rerender(mainLayoutElement({ selectedRootPath: "/managed/b" }));

		const paneA = screen.getByTestId("worktree-pane-/managed/a");
		const paneB = screen.getByTestId("worktree-pane-/managed/b");
		expect(
			within(paneA).getByTestId("node-content-view-mock"),
		).toBeInTheDocument();
		expect(paneA).toHaveAttribute("data-active", "false");
		expect(paneA).toHaveAttribute("aria-hidden", "true");
		expect(paneA).toHaveClass("invisible", "pointer-events-none");
		expect(paneB).toHaveAttribute("data-active", "true");
		expect(paneB).toHaveClass("visible");
		expect(paneB).not.toHaveClass("invisible", "pointer-events-none");
	});

	it("moves a re-selected pane back to the LRU head and shows it", () => {
		const view = renderMainLayout({ selectedRootPath: "/managed/a" });
		view.rerender(mainLayoutElement({ selectedRootPath: "/managed/b" }));
		view.rerender(mainLayoutElement({ selectedRootPath: "/managed/a" }));

		const paneA = screen.getByTestId("worktree-pane-/managed/a");
		expect(paneA).toHaveAttribute("data-active", "true");
		expect(paneA).toHaveClass("visible");
		expect(screen.getByTestId("worktree-pane-/managed/b")).toHaveAttribute(
			"data-active",
			"false",
		);
		const paneIds = screen
			.getAllByTestId(/^worktree-pane-/)
			.map((pane) => pane.getAttribute("data-testid"));
		expect(paneIds).toEqual([
			"worktree-pane-/managed/a",
			"worktree-pane-/managed/b",
		]);
	});

	it("unmounts only the least recently used pane beyond MAX_MOUNTED_PANES", () => {
		const paths = [
			"/managed/p1",
			"/managed/p2",
			"/managed/p3",
			"/managed/p4",
			"/managed/p5",
			"/managed/p6",
		];
		const view = renderMainLayout({ selectedRootPath: paths[0] });
		for (const path of paths.slice(1)) {
			view.rerender(mainLayoutElement({ selectedRootPath: path }));
		}

		expect(screen.queryByTestId("worktree-pane-/managed/p1")).toBeNull();
		for (const path of paths.slice(1)) {
			expect(screen.getByTestId(`worktree-pane-${path}`)).toBeInTheDocument();
		}
	});

	it("derives right panel visibility from the selected pane's own state", () => {
		const view = renderMainLayout({ selectedRootPath: "/managed/a" });
		const paneA = screen.getByTestId("worktree-pane-/managed/a");
		fireEvent.click(within(paneA).getByTestId("panel-right-collapse-trigger"));
		expect(
			within(paneA).getByTestId("right-panel-header-mock"),
		).toHaveAttribute("data-right-visible", "false");
		expect(rightSlotHasToggleFor("/managed/a")).toBe(true);

		view.rerender(mainLayoutElement({ selectedRootPath: "/managed/b" }));
		const paneB = screen.getByTestId("worktree-pane-/managed/b");
		expect(
			within(paneB).getByTestId("right-panel-header-mock"),
		).toHaveAttribute("data-right-visible", "true");
		expect(rightSlotHasToggleFor("/managed/b")).toBe(false);

		view.rerender(mainLayoutElement({ selectedRootPath: "/managed/a" }));
		expect(
			within(screen.getByTestId("worktree-pane-/managed/a")).getByTestId(
				"right-panel-header-mock",
			),
		).toHaveAttribute("data-right-visible", "false");
		expect(rightSlotHasToggleFor("/managed/a")).toBe(true);
	});
});
