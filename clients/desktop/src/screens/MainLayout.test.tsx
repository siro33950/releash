import { Code, ConnectError } from "@connectrpc/connect";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";

Element.prototype.scrollIntoView = vi.fn();

const mocks = vi.hoisted(() => ({
	nodeContentViewProps: vi.fn(),
	settingsModalProps: vi.fn(),
	branch: "feature" as string | null,
	branchError: null as string | null,
	baseError: null as string | null,
	baseBranch: "main" as string | null,
	localBranches: ["main", "feature"],
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
	useCurrentBranch: () => ({ branch: mocks.branch, error: mocks.branchError }),
}));
vi.mock("@/hooks/useBaseBranch", () => ({
	useBaseBranch: () => ({
		baseBranch: mocks.baseBranch,
		setBaseBranch: vi.fn(),
		localBranches: mocks.localBranches,
		error: mocks.baseError,
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
	SettingsModal: (props: unknown) => {
		mocks.settingsModalProps(props);
		return null;
	},
}));
vi.mock("@/screens/WorktreeViewDialogs", () => ({
	GitErrorDialog: () => null,
	CreateBranchDialog: () => null,
}));
vi.mock("@/components/layout/BranchSelector", async (importOriginal) => {
	const { BranchSelector } =
		await importOriginal<typeof import("@/components/layout/BranchSelector")>();
	return {
		BranchSelector: (props: React.ComponentProps<typeof BranchSelector>) => (
			<div data-testid="branch-selector-mock">
				<BranchSelector {...props} />
			</div>
		),
	};
});
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
					desktopSettingsLoaded={true}
					desktopSettingsError={null}
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

	it("worktreeの設定画面にも親の読み込み状態と失敗を渡す", () => {
		const view = renderMainLayout({
			desktopSettingsLoaded: false,
			desktopSettingsError: "settings unavailable",
		});
		expect(mocks.settingsModalProps).toHaveBeenLastCalledWith(
			expect.objectContaining({
				desktopSettingsLoaded: false,
				desktopSettingsError: "settings unavailable",
			}),
		);
		view.rerender(
			mainLayoutElement({
				desktopSettingsLoaded: true,
				desktopSettingsError: null,
			}),
		);
		expect(mocks.settingsModalProps).toHaveBeenLastCalledWith(
			expect.objectContaining({
				desktopSettingsLoaded: true,
				desktopSettingsError: null,
			}),
		);
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
			workspaceWorktreePath: "/managed/wt",
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

	it("Session作成の結果不明はOpening表示を置き換える", () => {
		const error = new ConnectError("Request failed", Code.Unavailable);
		renderMainLayout({
			centerSelectionByWorktree: {
				"/managed/wt": {
					kind: "agent_session_launching",
					worktreePath: "/managed/wt",
					provider: "codex",
					launchToken: "launch-1",
					error: error.message,
				},
			},
		});
		expect(
			screen.queryByText("Opening AgentSession..."),
		).not.toBeInTheDocument();
		expect(screen.getByRole("alert")).toHaveTextContent(error.message);
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

describe("ブランチ購読の失敗表示", () => {
	beforeEach(() => {
		mocks.branch = "feature";
		mocks.baseBranch = "main";
		mocks.localBranches = ["main", "feature"];
		mocks.branchError = null;
		mocks.baseError = null;
	});
	for (const target of ["current", "base", "choices"] as const) {
		it(`${target}の再読取失敗でも前回のブランチと選択肢を残す`, async () => {
			const user = userEvent.setup();
			const view = renderMainLayout();
			expect(screen.getByText("feature")).toBeVisible();
			expect(screen.getByRole("combobox")).toHaveTextContent("main");
			if (target === "current") mocks.branchError = "current denied";
			else mocks.baseError = `${target} denied`;
			view.rerender(mainLayoutElement());
			expect(screen.getByText("feature")).toBeVisible();
			expect(screen.getByRole("combobox")).toHaveTextContent("main");
			expect(screen.getByRole("alert")).toHaveTextContent(
				`showing previous values: ${target} denied`,
			);
			screen.getByRole("combobox").focus();
			await user.keyboard("{ArrowDown}");
			expect(screen.getByRole("option", { name: "feature" })).toBeVisible();
			expect(screen.getByRole("option", { name: "main" })).toBeVisible();
		});
	}
	it("未取得の失敗を正常な未設定と区別する", () => {
		mocks.branch = null;
		mocks.baseBranch = null;
		mocks.localBranches = [];
		mocks.branchError = "current denied";
		renderMainLayout();
		expect(screen.getByRole("alert")).toHaveTextContent(
			"Failed to read branch data: current denied",
		);
		expect(screen.getByRole("alert")).not.toHaveTextContent("showing previous");
		expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
	});
});
