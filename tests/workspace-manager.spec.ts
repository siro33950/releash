import { expect, test, type Locator } from "@playwright/test";
import {
	branchList,
	buildMockConfig,
	kanbanBranches,
} from "./helpers/fixtures";
import { emitTauriEvent, setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

async function waitForAnimations(locator: Locator) {
	await locator.evaluate(async (element) => {
		await Promise.all(
			element
				.getAnimations({ subtree: true })
				.map((animation) => animation.finished.catch(() => undefined)),
		);
	});
}

function rawSession(
	id: string,
	worktreePath: string,
	message: string,
	options: { pendingPermission?: boolean } = {},
) {
	return {
		id,
		worktreePath,
		messages: message
			? [
					{
						id: `${id}-message`,
						role: "agent",
						content: message,
						timestamp: 1000,
					},
				]
			: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		agentSessionId: `agent-${id}`,
		permissionMode: "ask",
		planMode: false,
		permissionProfileId: null,
		backendId: null,
		selectedModel: "",
		availableModels: [],
		canChangeBackend: false,
		pendingQueue: [],
		pendingQueueCount: 0,
		pendingPermissionRequest: options.pendingPermission
			? {
					id: "permission-workflow",
					toolName: "Bash",
					kind: "tool_approval",
					input: { command: "pnpm test" },
				}
			: null,
		pendingPermissionStateRevision: 1,
		turnPhase: options.pendingPermission ? "waiting_permission" : "idle",
		initialPage: { nextCursor: null, hasMore: false, totalCount: message ? 1 : 0 },
	};
}

function createdSession(id: string, worktreePath: string) {
	return {
		id,
		worktreePath,
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		agentSessionId: null,
		permissionMode: "ask",
		planMode: false,
		permissionProfileId: null,
		backendId: null,
	};
}

test.describe("Workspace Manager", () => {
	test("リポジトリが存在しない場合 No Repository が表示される", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_worktrees: [],
			list_branches_with_status: [],
			get_main_repo_path: { __mockError: "not a git repo" },
			get_repo_paths: [],
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("No Repository")).toBeVisible();
		await expect(
			page.getByRole("button", { name: "Add Repository" }),
		).toBeVisible();
	});

	test("worktree 付きブランチが WorkspaceList に表示される", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// useWorktreeList は worktree_path != null のブランチのみ表示する
		await expect(page.getByTestId("worktree-item-feat/wip")).toBeVisible();
		await expect(
			page.getByTestId("worktree-item-feat/review"),
		).toBeVisible();
	});

	test("worktree 付きブランチをクリックするとツリーが折りたたまれる", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [
					{
						kind: "node",
						id: "node-session-1",
						title: "Direct session",
						status: "running",
						contentKind: "session",
						capabilities: { canApprove: false, canClose: true },
						updatedAt: 1000,
					},
				],
				preferredNodeId: "node-session-1",
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("Direct session")).toBeVisible();
		await page.getByTestId("worktree-item-feat/wip").click();

		await expect(page.getByText("Direct session")).not.toBeVisible();
	});

	test("Workflow menu stays anchored to its trigger after hover out", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [
					{
						kind: "workflow",
						id: "workflow-opaque-1",
						title: "Release workflow",
						status: "running",
						capabilities: {
							canStop: true,
							canResume: false,
							canAbort: true,
							canArchive: false,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "node",
								id: "node-build-opaque",
								title: "build",
								status: "running",
								contentKind: "session",
								capabilities: { canApprove: false, canClose: false },
								updatedAt: 1000,
							},
						],
					},
				],
				preferredNodeId: "node-build-opaque",
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByText("Release workflow", { exact: true }).hover();
		const trigger = page.getByRole("button", {
			name: "Open menu for Release workflow",
		});
		await expect(trigger).toBeVisible();
		const triggerBox = await trigger.boundingBox();
		expect(triggerBox).not.toBeNull();
		await trigger.click();

		const menu = page.getByRole("menu").filter({ hasText: "Stop" });
		await expect(menu).toBeVisible();
		await waitForAnimations(menu);
		const menuBox = await menu.boundingBox();
		expect(menuBox).not.toBeNull();
		expect(menuBox!.x).toBeGreaterThan(10);
		expect(menuBox!.y).toBeGreaterThan(10);
		expect(
			Math.abs(
				menuBox!.x + menuBox!.width / 2 - (triggerBox!.x + triggerBox!.width / 2),
			),
		).toBeLessThan(160);
		expect(
			Math.abs(menuBox!.y - (triggerBox!.y + triggerBox!.height)),
		).toBeLessThan(40);

		await page.mouse.move(700, 500);
		await expect(menu).toBeVisible();
		const afterHoverOutBox = await menu.boundingBox();
		expect(afterHoverOutBox).not.toBeNull();
		expect(afterHoverOutBox!.x).toBeGreaterThan(10);
		expect(afterHoverOutBox!.y).toBeGreaterThan(10);
		expect(Math.abs(afterHoverOutBox!.x - menuBox!.x)).toBeLessThan(2);
		expect(Math.abs(afterHoverOutBox!.y - menuBox!.y)).toBeLessThan(2);
	});

	test("NewSession creates and selects a standalone Session Node", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const session = createdSession("session-new", worktreePath);
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			create_session: session,
			get_session: rawSession("session-new", worktreePath, ""),
			get_workspace_session_node_id: "node-new-opaque",
			get_workspace_node_detail: {
				id: "node-new-opaque",
				title: "New session",
				status: "running",
				capabilities: { canApprove: false, canClose: true },
				updatedAt: 1000,
				content: { kind: "session", sessionId: "session-new" },
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTestId("worktree-item-feat/wip").hover();
		await page.getByRole("button", { name: "Create in feat/wip" }).click();
		await page.getByRole("menuitem", { name: "NewSession" }).click();

		await expect(page.getByText("New session", { exact: true })).toBeVisible();
		await expect(page.getByPlaceholder("Send a message...")).toBeVisible();
		const lookup = await page.evaluate(() =>
			window.__TAURI_INTERNALS__?.invocations.find(
				(entry) => entry.cmd === "get_workspace_session_node_id",
			),
		);
		expect(lookup?.args).toEqual({
			worktreePath,
			sessionId: "session-new",
		});
	});

	test("Workflow Session uses the complete shared chat and permission surface", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [
					{
						kind: "workflow",
						id: "workflow-session-parent",
						title: "Review workflow",
						status: "waiting",
						capabilities: {
							canStop: false,
							canResume: false,
							canAbort: true,
							canArchive: false,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "node",
								id: "node-workflow-session",
								title: "Review changes",
								status: "waiting",
								contentKind: "session",
								capabilities: { canApprove: false, canClose: false },
								updatedAt: 1000,
							},
						],
					},
				],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				id: "node-workflow-session",
				title: "Review changes",
				status: "waiting",
				capabilities: { canApprove: false, canClose: false },
				updatedAt: 1000,
				content: { kind: "session", sessionId: "workflow-session" },
			},
			get_session: rawSession(
				"workflow-session",
				worktreePath,
				"Workflow session conversation body",
				{ pendingPermission: true },
			),
			present_agent_permission_request: null,
			respond_agent_permission: null,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page
			.getByRole("button", { name: "Review changes, waiting" })
			.click();

		await expect(
			page.getByText("Workflow session conversation body", { exact: true }),
		).toBeVisible();
		await expect(page.getByPlaceholder("Send a message...")).toBeVisible();
		await emitTauriEvent(page, "agent-streaming-delta", {
			chat_session_id: "workflow-session",
			message_id: "workflow-session-message",
			seq: 1,
			snapshot: true,
			parts: [{ type: "text", content: "Live workflow update" }],
		});
		await expect(page.getByText("Live workflow update", { exact: true })).toBeVisible();
		await page.getByRole("button", { name: "Allow", exact: true }).click();
		await expect
			.poll(async () =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.some(
						(entry) =>
							entry.cmd === "respond_agent_permission" &&
							entry.args.chatSessionId === "workflow-session" &&
							entry.args.requestId === "permission-workflow",
					),
				),
			)
			.toBe(true);
	});

	test("Fanout branch nests child Nodes and only toggles expansion", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [
					{
						kind: "workflow",
						id: "workflow-fanout-parent",
						title: "Fanout workflow",
						status: "running",
						capabilities: {
							canStop: true,
							canResume: false,
							canAbort: true,
							canArchive: false,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "fanout",
								id: "fanout-branch",
								title: "Matrix jobs",
								status: "running",
								updatedAt: 1000,
								children: [
									{
										kind: "node",
										id: "fanout-child-a",
										title: "Linux job",
										status: "running",
										contentKind: "command",
										capabilities: {
											canApprove: false,
											canClose: false,
										},
										updatedAt: 1000,
									},
								],
							},
						],
					},
				],
				preferredNodeId: null,
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		const fanout = page.getByRole("button", { name: "Matrix jobs" });
		await expect(page.getByText("Linux job", { exact: true })).toBeVisible();
		await fanout.click();
		await expect(page.getByText("Linux job", { exact: true })).not.toBeVisible();
		await fanout.click();
		await expect(page.getByText("Linux job", { exact: true })).toBeVisible();
		const detailCalls = await page.evaluate(() =>
			window.__TAURI_INTERNALS__?.invocations.filter(
				(entry) => entry.cmd === "get_workspace_node_detail",
			).length,
		);
		expect(detailCalls).toBe(0);
	});

	test("Workflow Command shows masked command and result outside the tree summary", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [
					{
						kind: "workflow",
						id: "workflow-command-parent",
						title: "Deploy workflow",
						status: "completed",
						capabilities: {
							canStop: false,
							canResume: false,
							canAbort: false,
							canArchive: true,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "fanout",
								id: "fanout-opaque",
								title: "Deploy batch",
								status: "completed",
								updatedAt: 1000,
								children: [
									{
										kind: "node",
										id: "node-command-opaque",
										title: "Deploy",
										status: "completed",
										contentKind: "command",
										capabilities: {
											canApprove: false,
											canClose: false,
										},
										updatedAt: 1000,
										nodeExecutionId: "internal-node-execution-uuid",
										attempt: 4,
									},
								],
							},
						],
					},
				],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				id: "node-command-opaque",
				title: "Deploy",
				status: "completed",
				capabilities: { canApprove: false, canClose: false },
				updatedAt: 1000,
				rawCommand: "deploy --token raw-super-secret",
				content: {
					kind: "command",
					displayCommand: "deploy --token ********",
					result: {
						exitCode: 0,
						duration: 245,
						stdout: "deploy complete",
						stderr: "masked warning",
					},
				},
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("Deploy batch", { exact: true })).toBeVisible();
		await expect(page.getByText("Deploy", { exact: true })).toBeVisible();
		await expect(page.getByText("deploy complete")).not.toBeVisible();
		await page.getByRole("button", { name: "Deploy batch" }).click();
		await expect(page.getByText("Deploy", { exact: true })).not.toBeVisible();
		await expect(page.getByText("Select a Node from the Workspace tree.")).not.toBeVisible();
		await page.getByRole("button", { name: "Deploy batch" }).click();
		await page
			.getByRole("button", { name: "Deploy, completed", exact: true })
			.click();

		await expect(page.getByTestId("workspace-command")).toContainText(
			"deploy --token ********",
		);
		await expect(page.getByText("245 ms", { exact: true })).toBeVisible();
		await expect(page.getByTestId("workspace-command-stdout")).toContainText(
			"deploy complete",
		);
		await expect(page.getByTestId("workspace-command-stderr")).toContainText(
			"masked warning",
		);
		await expect(page.getByText("raw-super-secret")).not.toBeVisible();
		await expect(page.getByText("internal-node-execution-uuid")).not.toBeVisible();
		await expect(page.getByText(/attempt 4/i)).not.toBeVisible();
	});

	test("retry refresh preserves the selected opaque Node while detail advances", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const nodeSummary = {
			kind: "node",
			id: "stable-retry-node",
			title: "Retryable review",
			status: "running",
			contentKind: "session",
			capabilities: { canApprove: false, canClose: false },
			updatedAt: 1000,
		};
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [nodeSummary],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				id: "stable-retry-node",
				title: "Retryable review",
				status: "running",
				capabilities: { canApprove: false, canClose: false },
				updatedAt: 1000,
				content: { kind: "session", sessionId: "retry-session-1" },
			},
			get_session: rawSession(
				"retry-session-1",
				worktreePath,
				"First attempt body",
			),
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		const selectedRow = page.getByRole("button", {
			name: "Retryable review, running",
			exact: true,
		});
		await selectedRow.click();
		await expect(page.getByText("First attempt body", { exact: true })).toBeVisible();
		await expect(selectedRow).toHaveAttribute("aria-current", "page");

		await page.evaluate(
			({ worktreePath, nodeSummary }) => {
				const internals = window.__TAURI_INTERNALS__;
				if (!internals) throw new Error("Tauri mock not initialized");
				internals.setMockResponse("list_workspace_worktree_nodes", {
					nodes: [{ ...nodeSummary, status: "completed", updatedAt: 2000 }],
					preferredNodeId: "stable-retry-node",
				});
				internals.setMockResponse("get_workspace_node_detail", {
					id: "stable-retry-node",
					title: "Retryable review",
					status: "completed",
					capabilities: { canApprove: false, canClose: false },
					updatedAt: 2000,
					content: { kind: "session", sessionId: "retry-session-2" },
				});
				internals.setMockResponse(
					"get_session",
					{
						id: "retry-session-2",
						worktreePath,
						messages: [
							{
								id: "retry-message-2",
								role: "agent",
								content: "Latest retry body",
								timestamp: 2000,
							},
						],
						state: "active",
						createdAt: 1000,
						updatedAt: 2000,
						agentSessionId: "agent-retry-2",
						permissionMode: "ask",
						planMode: false,
						permissionProfileId: null,
						backendId: null,
						selectedModel: "",
						availableModels: [],
						canChangeBackend: false,
						pendingQueue: [],
						pendingPermissionRequest: null,
						turnPhase: "idle",
						initialPage: {
							nextCursor: null,
							hasMore: false,
							totalCount: 1,
						},
					},
				);
				window.dispatchEvent(
					new CustomEvent("workspace-tree-refresh", {
						detail: { worktreePath },
					}),
				);
			},
			{ worktreePath, nodeSummary },
		);

		await expect(page.getByText("Latest retry body", { exact: true })).toBeVisible();
		const retriedRow = page.getByRole("button", {
			name: "Retryable review, completed",
			exact: true,
		});
		await expect(retriedRow).toHaveAttribute("aria-current", "page");
		await expect(page.getByText("First attempt body", { exact: true })).not.toBeVisible();
		await expect(page.getByText(/attempt 2/i)).not.toBeVisible();
	});
});

test.describe("CreateWorktreeModal", () => {
	test("Add worktree ボタンでモーダルが開く", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();

		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).toBeVisible();
	});

	test("Branch タブでブランチ一覧がフィルタリングされる", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();
		await expect(page.getByRole("heading", { name: "New Worktree" })).toBeVisible();

		const dialog = page.getByRole("dialog");

		// Branch タブに切り替え
		await dialog.getByRole("tab", { name: "Branch" }).click();

		// worktree なしブランチが表示される
		await expect(dialog.getByText("feat/todo")).toBeVisible();
		await expect(dialog.getByText("feat/done")).toBeVisible();

		// フィルター入力
		await dialog.getByPlaceholder("Filter branches...").fill("done");

		// feat/done のみ表示
		await expect(dialog.getByText("feat/done")).toBeVisible();
		await expect(dialog.getByText("feat/todo")).not.toBeVisible();
	});

	test("Cancel ボタンでモーダルが閉じる", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();
		await expect(page.getByRole("heading", { name: "New Worktree" })).toBeVisible();

		await page.getByRole("button", { name: "Cancel" }).click();

		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).not.toBeVisible();
	});

	test("Plain モードでブランチ名を入力すると Create が有効になる", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();
		await expect(page.getByRole("heading", { name: "New Worktree" })).toBeVisible();

		// Plain モードがデフォルト
		const branchInput = page.getByPlaceholder("feat/my-feature");
		await expect(branchInput).toBeVisible();

		// Create ボタンは初期状態で disabled
		const createBtn = page.getByRole("button", { name: "Create" });
		await expect(createBtn).toBeDisabled();

		// ブランチ名を入力
		await branchInput.fill("feat/new-feature");

		// Create ボタンが enabled になる
		await expect(createBtn).toBeEnabled();
	});
});
