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
			create_workspace_session: session.id,
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
		const creation = await page.evaluate(() =>
			window.__TAURI_INTERNALS__?.invocations.find(
				(entry) => entry.cmd === "create_workspace_session",
			),
		);
		expect(creation?.args).toEqual({
			requestId: expect.any(String),
			worktreePath,
			permissionMode: "edit",
			backendId: null,
			modelId: null,
		});
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

	test("closing the selected standalone Session clears the center", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const node = {
			kind: "node",
			id: "node-direct-opaque",
			title: "Direct session",
			status: "running",
			contentKind: "session",
			capabilities: { canApprove: false, canClose: true },
			updatedAt: 1000,
		};
		const detail = {
			id: node.id,
			title: node.title,
			status: node.status,
			capabilities: node.capabilities,
			updatedAt: node.updatedAt,
			content: { kind: "session", sessionId: "session-direct" },
		};
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [node],
				preferredNodeId: node.id,
			},
			get_workspace_node_detail: detail,
			get_session: rawSession("session-direct", worktreePath, "Before close"),
			close_workspace_node: null,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		await page
			.getByRole("button", { name: "Direct session, running" })
			.click();
		await expect(page.getByText("Before close", { exact: true })).toBeVisible();

		await page.evaluate(() => {
			window.__TAURI_INTERNALS__?.setMockResponse(
				"list_workspace_worktree_nodes",
				{ nodes: [], preferredNodeId: null },
			);
			window.__TAURI_INTERNALS__?.setMockResponse(
				"get_workspace_node_detail",
				null,
			);
		});
		await page
			.getByRole("button", { name: "Close Direct session" })
			.click();

		await expect(
			page.getByText("Select a Node from the Workspace tree."),
		).toBeVisible();
		await expect(
			page.getByRole("button", { name: /Direct session/ }),
		).toHaveCount(0);
		const invocations = await page.evaluate(
			() => window.__TAURI_INTERNALS__?.invocations ?? [],
		);
		expect(invocations).toContainEqual({
			cmd: "close_workspace_node",
			args: { worktreePath, nodeId: node.id },
		});
		expect(invocations.some((entry) => entry.cmd === "close_session")).toBe(
			false,
		);
	});

	test("the first Workflow Node is selected after an initially empty snapshot", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const workflowNode = {
			kind: "node",
			id: "node-first-workflow-opaque",
			title: "First workflow Session",
			status: "running",
			contentKind: "session",
			capabilities: { canApprove: false, canClose: false },
			updatedAt: 1000,
		};
		const firstWorkflowSession = rawSession(
			"session-first-workflow",
			worktreePath,
			"First workflow transcript",
		);
		const config = buildMockConfig({
			list_worktrees: [{ path: worktreePath, branch: "feat/wip" }],
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [],
				preferredNodeId: null,
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		await expect(page.getByText("No sessions or workflows")).toBeVisible();

		await page.evaluate(
			({ worktreePath, workflowNode, firstWorkflowSession }) => {
				window.__TAURI_INTERNALS__?.setMockResponse(
					"list_workspace_worktree_nodes",
					{
						nodes: [
							{
								kind: "workflow",
								id: "workflow-first",
								title: "First workflow",
								status: "running",
								capabilities: {
									canStop: true,
									canResume: false,
									canAbort: true,
									canArchive: false,
								},
								children: [workflowNode],
								updatedAt: 1000,
							},
						],
						preferredNodeId: workflowNode.id,
					},
				);
				window.__TAURI_INTERNALS__?.setMockResponse(
					"get_workspace_node_detail",
					{
						id: workflowNode.id,
						title: workflowNode.title,
						status: workflowNode.status,
						capabilities: workflowNode.capabilities,
						updatedAt: workflowNode.updatedAt,
						content: {
							kind: "session",
							sessionId: "session-first-workflow",
						},
					},
				);
				window.__TAURI_INTERNALS__?.setMockResponse(
					"get_session",
					firstWorkflowSession,
				);
				window.dispatchEvent(
					new CustomEvent("workspace-tree-refresh", {
						detail: { worktreePath },
					}),
				);
			},
			{
				worktreePath,
				workflowNode,
				firstWorkflowSession,
			},
		);

		await expect(
			page.getByText("First workflow transcript", { exact: true }),
		).toBeVisible();
		await expect(page.getByPlaceholder("Send a message...")).toBeVisible();
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

	test("Codex send直後のStopは再押下できqueueとdraftを保持してpausedで終端する", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const sessionId = "codex-stop-session";
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [
					{
						kind: "node",
						id: "node-codex-stop",
						title: "Codex stop",
						status: "running",
						contentKind: "session",
						capabilities: { canApprove: false, canClose: false },
						updatedAt: 1000,
					},
				],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				id: "node-codex-stop",
				title: "Codex stop",
				status: "running",
				capabilities: { canApprove: false, canClose: false },
				updatedAt: 1000,
				content: { kind: "session", sessionId },
			},
			get_session: {
				...rawSession(sessionId, worktreePath, "Starting Codex turn"),
				backendId: "codex",
				turnPhase: "streaming",
				pendingQueue: [
					{
						id: "queued-follow-up",
						contentPreview: "queued follow-up",
						createdAt: 1001,
						permissionMode: "edit",
						imageCount: 1,
					},
				],
				pendingQueueCount: 1,
				queuePaused: false,
			},
			interrupt_agent_query: null,
			resume_agent_queue: null,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		await page.getByRole("button", { name: "Codex stop, running" }).click();
		const composer = page.getByPlaceholder("Send a message...");
		await composer.fill("draft must survive stop");

		await page.getByRole("button", { name: "Interrupt agent" }).click();
		await page.getByRole("button", { name: "Stopping agent" }).click();
		await expect
			.poll(() =>
				page.evaluate(
					() =>
						window.__TAURI_INTERNALS__?.invocations.filter(
							(entry) => entry.cmd === "interrupt_agent_query",
						).length ?? 0,
				),
			)
			.toBe(2);

		await emitTauriEvent(page, "agent-session-state-changed", {
			chat_session_id: sessionId,
			turn_phase: "idle",
			exit_code: 1,
			completed_at: 1010,
			interrupted: true,
			session_state: "error",
			queue_paused: true,
			pending_permission_request: null,
			pending_permission_state_revision: 2,
		});

		await expect(composer).toHaveValue("draft must survive stop");
		await expect(page.getByText("queued follow-up", { exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Resume queue" })).toBeVisible();
		await page.getByRole("button", { name: "Resume queue" }).click();
		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.some(
						(entry) => entry.cmd === "resume_agent_queue",
					) ?? false,
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

	test("a later occurrence appends without replacing the selected past occurrence", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const firstOccurrence = {
			kind: "node",
			id: "occurrence-a-1",
			title: "Loop step",
			status: "running",
			contentKind: "session",
			capabilities: { canApprove: false, canClose: false },
			updatedAt: 1000,
		};
		const workflowSummary = {
			kind: "workflow",
			id: "loop-workflow",
			title: "Loop workflow",
			status: "running",
			capabilities: {
				canStop: true,
				canResume: false,
				canAbort: true,
				canArchive: false,
			},
			updatedAt: 1000,
			children: [firstOccurrence],
		};
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [workflowSummary],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				id: "occurrence-a-1",
				title: "Loop step",
				status: "running",
				capabilities: { canApprove: false, canClose: false },
				updatedAt: 1000,
				content: { kind: "session", sessionId: "loop-session-a-1" },
			},
			get_session: rawSession(
				"loop-session-a-1",
				worktreePath,
				"First occurrence body",
			),
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		const firstRow = page.getByRole("button", {
			name: "Loop step, running",
			exact: true,
		});
		await firstRow.click();
		await expect(
			page.getByText("First occurrence body", { exact: true }),
		).toBeVisible();
		await expect(firstRow).toHaveAttribute("aria-current", "page");

		await page.evaluate(
			({ worktreePath, workflowSummary, firstOccurrence }) => {
				const internals = window.__TAURI_INTERNALS__;
				if (!internals) throw new Error("Tauri mock not initialized");
				const completedFirst = {
					...firstOccurrence,
					status: "completed",
					updatedAt: 2000,
				};
				const secondOccurrence = {
					...firstOccurrence,
					id: "occurrence-a-2",
					status: "running",
					updatedAt: 3000,
				};
				internals.setMockResponse("list_workspace_worktree_nodes", {
					nodes: [
						{
							...workflowSummary,
							updatedAt: 3000,
							children: [completedFirst, secondOccurrence],
						},
					],
					preferredNodeId: "occurrence-a-2",
				});
				internals.setMockResponse("get_workspace_node_detail", {
					id: "occurrence-a-1",
					title: "Loop step",
					status: "completed",
					capabilities: { canApprove: false, canClose: false },
					updatedAt: 2000,
					content: { kind: "session", sessionId: "loop-session-a-1" },
				});
				window.dispatchEvent(
					new CustomEvent("workspace-tree-refresh", {
						detail: { worktreePath },
					}),
				);
			},
			{ worktreePath, workflowSummary, firstOccurrence },
		);

		const completedFirstRow = page.getByRole("button", {
			name: "Loop step, completed",
			exact: true,
		});
		const secondRow = page.getByRole("button", {
			name: "Loop step, running",
			exact: true,
		});
		await expect(completedFirstRow).toHaveAttribute("aria-current", "page");
		await expect(secondRow).not.toHaveAttribute("aria-current");
		await expect(
			page.getByText("First occurrence body", { exact: true }),
		).toBeVisible();

		await page.evaluate(
			({ worktreePath }) => {
				const internals = window.__TAURI_INTERNALS__;
				if (!internals) throw new Error("Tauri mock not initialized");
				internals.setMockResponse("get_workspace_node_detail", {
					id: "occurrence-a-2",
					title: "Loop step",
					status: "running",
					capabilities: { canApprove: false, canClose: false },
					updatedAt: 3000,
					content: { kind: "session", sessionId: "loop-session-a-2" },
				});
				internals.setMockResponse("get_session", {
					id: "loop-session-a-2",
					worktreePath,
					messages: [
						{
							id: "loop-message-a-2",
							role: "agent",
							content: "Second occurrence body",
							timestamp: 3000,
						},
					],
					state: "active",
					createdAt: 3000,
					updatedAt: 3000,
					agentSessionId: "agent-loop-a-2",
					permissionMode: "ask",
					planMode: false,
					permissionProfileId: null,
					backendId: null,
					selectedModel: "",
					availableModels: [],
					canChangeBackend: false,
					pendingQueue: [],
					pendingQueueCount: 0,
					pendingPermissionRequest: null,
					pendingPermissionStateRevision: 1,
					turnPhase: "idle",
					initialPage: {
						nextCursor: null,
						hasMore: false,
						totalCount: 1,
					},
				});
			},
			{ worktreePath },
		);
		await secondRow.click();

		await expect(
			page.getByText("Second occurrence body", { exact: true }),
		).toBeVisible();
		await expect(secondRow).toHaveAttribute("aria-current", "page");
		await expect(
			page.getByText("First occurrence body", { exact: true }),
		).not.toBeVisible();
		await expect(page.getByText(/attempt/i)).not.toBeVisible();
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
