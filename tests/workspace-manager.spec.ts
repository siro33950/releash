import { expect, test, type Locator } from "@playwright/test";
import {
	branchList,
	buildMockConfig,
	kanbanBranches,
} from "./helpers/fixtures";
import {
	setupTauriMock,
	workspaceTreeReconciliation,
} from "./helpers/tauri-mock";
import {
	waitForApp,
	waitForWorkspaceTreeQuiescence,
} from "./helpers/utils";

async function waitForAnimations(locator: Locator) {
	await locator.evaluate(async (element) => {
		await Promise.all(
			element
				.getAnimations({ subtree: true })
				.map((animation) => animation.finished.catch(() => undefined)),
		);
	});
}

function agentSession(id: string, worktreePath: string) {
	return {
		id,
		workspaceIdentity: worktreePath,
		worktreePath,
		provider: "codex",
		lifecycle: "open",
		activity: "idle",
		lastExitAbnormal: false,
		operations: {
			canArchive: true,
			canRestore: false,
			canDelete: false,
		},
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

	test("NewSession requires a Provider and opens the created TUI AgentSession", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const agentSessionId = "agent-session-new";
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_available_agent_session_providers: ["codex"],
			create_agent_session: agentSessionId,
			get_agent_session: agentSession(agentSessionId, worktreePath),
			open_agent_session: "attached",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTestId("worktree-item-feat/wip").hover();
		await page.getByRole("button", { name: "Create in feat/wip" }).click();
		await page.getByRole("menuitem", { name: "NewSession" }).click();
		await page.getByRole("menuitem", { name: "codex" }).click();
		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.find(
						(entry) =>
							entry.cmd === "attach_terminal_surface" &&
							entry.args.owner?.kind === "session",
					),
				),
			)
			.toMatchObject({
				args: {
					owner: {
						kind: "session",
						workspacePath: worktreePath,
						sessionId: agentSessionId,
					},
				},
			});
		const creation = await page.evaluate(() =>
			window.__TAURI_INTERNALS__?.invocations.find(
				(entry) => entry.cmd === "create_agent_session",
			),
		);
		expect(creation?.args).toEqual({
			workspaceIdentity: worktreePath,
			worktreePath,
			provider: "codex",
			rows: 24,
			cols: 80,
			callerRequestId: expect.any(String),
		});
		const invocations = await page.evaluate(
			() => window.__TAURI_INTERNALS__?.invocations ?? [],
		);
		expect(invocations).toContainEqual({
			cmd: "get_agent_session",
			args: { agentSessionId },
		});
		expect(
			invocations.some((entry) => entry.cmd === "open_agent_session"),
		).toBe(false);
		expect(invocations.some((entry) => entry.cmd === "create_workspace_session"))
			.toBe(false);
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
		const firstWorkflowSession = agentSession(
			"agent-session-first-workflow",
			worktreePath,
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
							kind: "agentSession",
							sessionId: "agent-session-first-workflow",
						},
					},
				);
				window.__TAURI_INTERNALS__?.setMockResponse(
					"get_agent_session",
					firstWorkflowSession,
				);
				window.__TAURI_INTERNALS__?.setMockResponse(
					"open_agent_session",
					"attached",
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

		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.find(
						(entry) =>
							entry.cmd === "attach_terminal_surface" &&
							entry.args.owner?.kind === "session",
					),
				),
			)
			.toMatchObject({
				args: {
					owner: {
						kind: "session",
						workspacePath: worktreePath,
						sessionId: firstWorkflowSession.id,
					},
				},
			});
	});

	test("Workflow Session uses the AgentSession TUI without legacy Message or permission commands", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const agentSessionId = "agent-session-workflow";
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
				content: { kind: "agentSession", sessionId: agentSessionId },
			},
			get_agent_session: agentSession(agentSessionId, worktreePath),
			open_agent_session: "attached",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page
			.getByRole("button", { name: "Review changes, waiting" })
			.click();

		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.find(
						(entry) =>
							entry.cmd === "attach_terminal_surface" &&
							entry.args.owner?.kind === "session",
					),
				),
			)
			.toMatchObject({
				args: {
					owner: {
						kind: "session",
						workspacePath: worktreePath,
						sessionId: agentSessionId,
					},
				},
			});
		const legacyCommands = new Set([
			"get_session",
			"get_agent_session_display_window",
			"present_agent_permission_request",
			"report_agent_permission_request_observed",
			"respond_agent_permission",
		]);
		const invocations = await page.evaluate(
			() => window.__TAURI_INTERNALS__?.invocations ?? [],
		);
		expect(invocations.some(({ cmd }) => legacyCommands.has(cmd))).toBe(false);
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

	test("Archive reconciliation derives membership from its opt-in response snapshot", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const selectedNodeId = "archive-selected-node";
		const fallbackNodeId = "archive-fallback-node";
		const initialSnapshot = {
			nodes: [
				{
					kind: "node",
					id: fallbackNodeId,
					title: "Archive fallback",
					status: "running",
					contentKind: "session",
					capabilities: { canApprove: false, canClose: true },
					updatedAt: 1000,
				},
				{
					kind: "workflow",
					id: "archivable-workflow",
					title: "Archivable integration workflow",
					status: "completed",
					capabilities: {
						canStop: false,
						canResume: false,
						canAbort: false,
						canArchive: true,
					},
					updatedAt: 2000,
					children: [
						{
							kind: "node",
							id: selectedNodeId,
							title: "Archive selected",
							status: "completed",
							contentKind: "session",
							capabilities: { canApprove: false, canClose: false },
							updatedAt: 2000,
						},
					],
				},
			],
			preferredNodeId: null,
		};
		const reconciledSnapshot = {
			nodes: [initialSnapshot.nodes[0]],
			preferredNodeId: fallbackNodeId,
		};
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: initialSnapshot,
			get_workspace_tree_selection_reconciliation:
				workspaceTreeReconciliation(reconciledSnapshot),
			archive_workspace_workflow_execution: null,
			get_workspace_node_detail: {
				id: selectedNodeId,
				title: "Archive selected",
				status: "completed",
				capabilities: { canApprove: false, canClose: false },
				updatedAt: 2000,
				content: {
					kind: "agentSession",
					sessionId: "agent-session-archive-selected",
				},
			},
			get_agent_session: agentSession(
				"agent-session-archive-selected",
				worktreePath,
			),
			open_agent_session: "attached",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page
			.getByRole("button", { name: "Archive selected, completed" })
			.click();
		await expect(
			page.getByRole("button", { name: "Archive selected, completed" }),
		).toHaveAttribute("aria-current", "page");
		await page
			.getByRole("button", { name: "Archivable integration workflow" })
			.hover();
		await page
			.getByRole("button", { name: "Archive Archivable integration workflow" })
			.click();

		await expect(
			page.getByRole("button", { name: "Archive fallback, running" }),
		).toHaveAttribute("aria-current", "page");
		const reconciliationInvocations = await page.evaluate(
			() =>
				window.__TAURI_INTERNALS__?.invocations.filter(
					(entry) =>
						entry.cmd === "get_workspace_tree_selection_reconciliation",
				) ?? [],
		);
		expect(reconciliationInvocations).toEqual([
			{
				cmd: "get_workspace_tree_selection_reconciliation",
				args: { worktreePath, selectedNodeId },
			},
		]);
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
				content: {
					kind: "agentSession",
					sessionId: "agent-session-loop-a-1",
				},
			},
			get_agent_session: agentSession(
				"agent-session-loop-a-1",
				worktreePath,
			),
			open_agent_session: "attached",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		const firstRow = page.getByRole("button", {
			name: "Loop step, running",
			exact: true,
		});
		await firstRow.click();
		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.find(
						(entry) =>
							entry.cmd === "attach_terminal_surface" &&
							entry.args.owner?.sessionId === "agent-session-loop-a-1",
					),
				),
			)
			.toBeTruthy();
		const refreshInvocations = await page.evaluate(
			() => window.__TAURI_INTERNALS__?.invocations ?? [],
		);
		expect(
			refreshInvocations.filter(
				(entry) =>
					entry.cmd === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(0);
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
					content: {
						kind: "agentSession",
						sessionId: "agent-session-loop-a-1",
					},
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
		const updateInvocations = await page.evaluate(
			() => window.__TAURI_INTERNALS__?.invocations ?? [],
		);
		expect(
			updateInvocations.filter(
				(entry) =>
					entry.cmd === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(0);

		const secondSession = agentSession(
			"agent-session-loop-a-2",
			worktreePath,
		);
		await page.evaluate(
			({ worktreePath, secondSession }) => {
				const internals = window.__TAURI_INTERNALS__;
				if (!internals) throw new Error("Tauri mock not initialized");
				internals.setMockResponse("get_workspace_node_detail", {
					id: "occurrence-a-2",
					title: "Loop step",
					status: "running",
					capabilities: { canApprove: false, canClose: false },
					updatedAt: 3000,
					content: {
						kind: "agentSession",
						sessionId: "agent-session-loop-a-2",
					},
				});
				internals.setMockResponse("get_agent_session", secondSession);
			},
			{ worktreePath, secondSession },
		);
		await secondRow.click();

		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__TAURI_INTERNALS__?.invocations.find(
						(entry) =>
							entry.cmd === "attach_terminal_surface" &&
							entry.args.owner?.sessionId === "agent-session-loop-a-2",
					),
				),
			)
			.toBeTruthy();
		await expect(secondRow).toHaveAttribute("aria-current", "page");
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
