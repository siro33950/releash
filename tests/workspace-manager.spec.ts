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
import { waitForApp, waitForWorkspaceTreeQuiescence } from "./helpers/utils";

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
		workspaceWorktreePath: worktreePath,
		treeLocation: { treeId: "mock-tree", nodeExecutionId: id },
		worktreePath,
		provider: "codex",
		lifecycle: "open",
		lastExitAbnormal: false,
		providerSessionId: null,
		transcriptRef: null,
		operations: {
			canArchive: true,
			canRestore: false,
			canDelete: false,
			canResume: false,
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

		// backend の一覧には worktree_path を持つブランチだけを含める
		await expect(page.getByTestId("worktree-item-feat/wip")).toBeVisible();
		await expect(page.getByTestId("worktree-item-feat/review")).toBeVisible();
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
						processPresence: "unknown",
						id: "node-session-1",
						title: "Direct session",
						status: "active",
						contentKind: "session",
						capabilities: {
							canRename: false,
							canApprove: false,
							canRetry: false, canResumeSession: false,
						},
						pastAttempts: [],
						pastAttemptsCollapsed: false,
						updatedAt: 1000,
					},
				],
				archivedSessions: [],
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
						kind: "sequence",
						id: "workflow-opaque-1",
						title: "Release workflow",
						status: "active",
						workflowCapabilities: {
							canAbort: true,
							canArchive: false,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "node",
								processPresence: "unknown",
								id: "node-build-opaque",
								title: "build",
								status: "active",
								contentKind: "session",
								capabilities: {
									canRename: false,
									canApprove: false,
									canRetry: false, canResumeSession: false,
								},
								pastAttempts: [],
								pastAttemptsCollapsed: false,
								updatedAt: 1000,
							},
						],
					},
				],
				archivedSessions: [],
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

		const menu = page.getByRole("menu").filter({ hasText: "Abort" });
		await expect(menu).toBeVisible();
		await waitForAnimations(menu);
		const menuBox = await menu.boundingBox();
		expect(menuBox).not.toBeNull();
		expect(menuBox!.x).toBeGreaterThan(10);
		expect(menuBox!.y).toBeGreaterThan(10);
		expect(
			Math.abs(
				menuBox!.x +
					menuBox!.width / 2 -
					(triggerBox!.x + triggerBox!.width / 2),
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
			get_workspace_session_node_id: agentSessionId,
			get_workspace_node_detail: {
				processPresence: "unknown",
				statusClassification: "active",
				id: agentSessionId,
				title: "New Session",
				status: "running",
				capabilities: {
					canRename: false,
					canApprove: false,
					canRetry: false, canResumeSession: false,
				},
				updatedAt: 1000,
				submitReceived: false,
				stopReceived: false,
				hasArtifact: false,
				content: { kind: "session", sessionId: agentSessionId },
			},
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
					window.__RELEASH_BACKEND__?.invocations.find(
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
			window.__RELEASH_BACKEND__?.invocations.find(
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
			() => window.__RELEASH_BACKEND__?.invocations ?? [],
		);
		expect(invocations).toContainEqual({
			cmd: "get_agent_session",
			args: { agentSessionId },
		});
		expect(
			invocations.some((entry) => entry.cmd === "open_agent_session"),
		).toBe(false);
		expect(
			invocations.some((entry) => entry.cmd === "create_workspace_session"),
		).toBe(false);
	});

	test("the first Workflow Node is selected after an initially empty snapshot", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const workflowNode = {
			kind: "node",
			processPresence: "unknown",
			id: "node-first-workflow-opaque",
			title: "First workflow Session",
			status: "active",
			contentKind: "session",
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: false, canResumeSession: false,
			},
			pastAttempts: [],
			pastAttemptsCollapsed: false,
			updatedAt: 1000,
		};
		const firstWorkflowSession = agentSession(
			"agent-session-first-workflow",
			worktreePath,
		);
		const config = buildMockConfig({
			list_worktrees: [
				{
					name: "feat-wip",
					is_main: false,
					is_locked: false,
					dirty_count: 0,
					base_branch: "main",
					path: worktreePath,
					branch: "feat/wip",
				},
			],
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: {
				nodes: [],
				archivedSessions: [],
				preferredNodeId: null,
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		await expect(page.getByText("No sessions or workflows")).toBeVisible();

		await page.evaluate(
			({ worktreePath, workflowNode, firstWorkflowSession }) => {
				window.__RELEASH_BACKEND__?.setMockResponse(
					"list_workspace_worktree_nodes",
					{
						nodes: [
							{
								kind: "sequence",
								id: "workflow-first",
								title: "First workflow",
								status: "active",
								workflowCapabilities: {
									canAbort: true,
									canArchive: false,
								},
								children: [workflowNode],
								updatedAt: 1000,
							},
						],
						archivedSessions: [],
						preferredNodeId: workflowNode.id,
					},
				);
				window.__RELEASH_BACKEND__?.setMockResponse(
					"get_workspace_node_detail",
					{
						processPresence: "unknown",
						statusClassification: "active",
						id: workflowNode.id,
						title: workflowNode.title,
						status: "running",
						capabilities: workflowNode.capabilities,
						updatedAt: workflowNode.updatedAt,
						submitReceived: false,
						stopReceived: false,
						hasArtifact: false,
						content: {
							kind: "session",
							sessionId: "agent-session-first-workflow",
						},
					},
				);
				window.__RELEASH_BACKEND__?.setMockResponse(
					"get_agent_session",
					firstWorkflowSession,
				);
				window.__RELEASH_BACKEND__?.setMockResponse(
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
					window.__RELEASH_BACKEND__?.invocations.find(
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
						kind: "sequence",
						id: "workflow-session-parent",
						title: "Review workflow",
						status: "attention",
						workflowCapabilities: {
							canAbort: true,
							canArchive: false,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "node",
								processPresence: "unknown",
								id: "node-workflow-session",
								title: "Review changes",
								status: "attention",
								contentKind: "session",
								capabilities: {
									canRename: false,
									canApprove: false,
									canRetry: false, canResumeSession: false,
								},
								pastAttempts: [],
								pastAttemptsCollapsed: false,
								updatedAt: 1000,
							},
						],
					},
				],
				archivedSessions: [],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				processPresence: "unknown",
				statusClassification: "attention",
				id: "node-workflow-session",
				title: "Review changes",
				status: "waiting",
				capabilities: {
					canRename: false,
					canApprove: false,
					canRetry: false, canResumeSession: false,
				},
				updatedAt: 1000,
				submitReceived: false,
				stopReceived: false,
				hasArtifact: false,
				content: { kind: "session", sessionId: agentSessionId },
			},
			get_agent_session: agentSession(agentSessionId, worktreePath),
			open_agent_session: "attached",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page
			.getByRole("button", { name: "Review changes, attention" })
			.click();

		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__RELEASH_BACKEND__?.invocations.find(
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
			() => window.__RELEASH_BACKEND__?.invocations ?? [],
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
						kind: "sequence",
						id: "workflow-fanout-parent",
						title: "Fanout workflow",
						status: "active",
						workflowCapabilities: {
							canAbort: true,
							canArchive: false,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "fanout",
								id: "fanout-branch",
								title: "Matrix jobs",
								status: "active",
								updatedAt: 1000,
								children: [
									{
										kind: "node",
										processPresence: "unknown",
										id: "fanout-child-a",
										title: "Linux job",
										status: "active",
										contentKind: "command",
										capabilities: {
											canRename: false,
											canApprove: false,
											canRetry: false, canResumeSession: false,
										},
										pastAttempts: [],
										pastAttemptsCollapsed: false,
										updatedAt: 1000,
									},
								],
							},
						],
					},
				],
				archivedSessions: [],
				preferredNodeId: null,
			},
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		const fanout = page.getByRole("button", { name: "Matrix jobs" });
		await expect(page.getByText("Linux job", { exact: true })).toBeVisible();
		await fanout.click();
		await expect(
			page.getByText("Linux job", { exact: true }),
		).not.toBeVisible();
		await fanout.click();
		await expect(page.getByText("Linux job", { exact: true })).toBeVisible();
		const detailCalls = await page.evaluate(
			() =>
				window.__RELEASH_BACKEND__?.invocations.filter(
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
						kind: "sequence",
						id: "workflow-command-parent",
						title: "Deploy workflow",
						status: "idle",
						workflowCapabilities: {
							canAbort: false,
							canArchive: true,
						},
						updatedAt: 1000,
						children: [
							{
								kind: "fanout",
								id: "fanout-opaque",
								title: "Deploy batch",
								status: "idle",
								updatedAt: 1000,
								children: [
									{
										kind: "node",
										processPresence: "unknown",
										id: "node-command-opaque",
										title: "Deploy",
										status: "idle",
										contentKind: "command",
										capabilities: {
											canRename: false,
											canApprove: false,
											canRetry: false, canResumeSession: false,
										},
										pastAttempts: [],
										pastAttemptsCollapsed: false,
										updatedAt: 1000,
									},
								],
							},
						],
					},
				],
				archivedSessions: [],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				processPresence: "unknown",
				statusClassification: "idle",
				id: "node-command-opaque",
				title: "Deploy",
				status: "completed",
				capabilities: {
					canRename: false,
					canApprove: false,
					canRetry: false, canResumeSession: false,
				},
				updatedAt: 1000,
				submitReceived: false,
				stopReceived: false,
				hasArtifact: false,
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
		await expect(
			page.getByText("Select a Node from the Workspace tree."),
		).not.toBeVisible();
		await page.getByRole("button", { name: "Deploy batch" }).click();
		await page
			.getByRole("button", { name: "Deploy, idle", exact: true })
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
		await expect(
			page.getByText("internal-node-execution-uuid"),
		).not.toBeVisible();
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
					processPresence: "unknown",
					id: fallbackNodeId,
					title: "Archive fallback",
					status: "active",
					contentKind: "session",
					capabilities: {
						canRename: false,
						canApprove: false,
						canRetry: false, canResumeSession: false,
					},
					pastAttempts: [],
					pastAttemptsCollapsed: false,
					updatedAt: 1000,
				},
				{
					kind: "sequence",
					id: "archivable-workflow",
					title: "Archivable integration workflow",
					status: "idle",
					workflowCapabilities: {
						canAbort: false,
						canArchive: true,
					},
					updatedAt: 2000,
					children: [
						{
							kind: "node",
							processPresence: "unknown",
							id: selectedNodeId,
							title: "Archive selected",
							status: "idle",
							contentKind: "session",
							capabilities: {
								canRename: false,
								canApprove: false,
								canRetry: false, canResumeSession: false,
							},
							pastAttempts: [],
							pastAttemptsCollapsed: false,
							updatedAt: 2000,
						},
					],
				},
			],
			archivedSessions: [],
			preferredNodeId: null,
		};
		const reconciledSnapshot = {
			nodes: [initialSnapshot.nodes[0]],
			archivedSessions: [],
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
				processPresence: "unknown",
				statusClassification: "idle",
				id: selectedNodeId,
				title: "Archive selected",
				status: "completed",
				capabilities: {
					canRename: false,
					canApprove: false,
					canRetry: false, canResumeSession: false,
				},
				updatedAt: 2000,
				submitReceived: false,
				stopReceived: false,
				hasArtifact: false,
				content: {
					kind: "session",
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

		await page.getByRole("button", { name: "Archive selected, idle" }).click();
		await expect(
			page.getByRole("button", { name: "Archive selected, idle" }),
		).toHaveAttribute("aria-current", "page");
		await page
			.getByRole("button", {
				name: "Archivable integration workflow",
				exact: true,
			})
			.hover();
		await page.evaluate((snapshot) => {
			window.__RELEASH_BACKEND__?.setMockResponse(
				"list_workspace_worktree_nodes",
				snapshot,
			);
		}, reconciledSnapshot);

		await page
			.getByRole("button", { name: "Archive Archivable integration workflow" })
			.click();

		await expect(
			page.getByRole("button", { name: "Archive fallback, active" }),
		).toHaveAttribute("aria-current", "page");
		const reconciliationInvocations = await page.evaluate(
			() =>
				window.__RELEASH_BACKEND__?.invocations.filter(
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
			processPresence: "unknown",
			id: "occurrence-a-1",
			title: "Loop step",
			status: "active",
			contentKind: "session",
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: false, canResumeSession: false,
			},
			pastAttempts: [],
			pastAttemptsCollapsed: false,
			updatedAt: 1000,
		};
		const workflowSummary = {
			kind: "sequence",
			id: "loop-workflow",
			title: "Loop workflow",
			status: "active",
			workflowCapabilities: {
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
				archivedSessions: [],
				preferredNodeId: null,
			},
			get_workspace_node_detail: {
				processPresence: "unknown",
				statusClassification: "active",
				id: "occurrence-a-1",
				title: "Loop step",
				status: "running",
				capabilities: {
					canRename: false,
					canApprove: false,
					canRetry: false, canResumeSession: false,
				},
				updatedAt: 1000,
				submitReceived: false,
				stopReceived: false,
				hasArtifact: false,
				content: {
					kind: "session",
					sessionId: "agent-session-loop-a-1",
				},
			},
			get_agent_session: agentSession("agent-session-loop-a-1", worktreePath),
			open_agent_session: "attached",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);
		const firstRow = page.getByRole("button", {
			name: "Loop step, active",
			exact: true,
		});
		await firstRow.click();
		await expect
			.poll(() =>
				page.evaluate(() =>
					window.__RELEASH_BACKEND__?.invocations.find(
						(entry) =>
							entry.cmd === "attach_terminal_surface" &&
							entry.args.owner?.sessionId === "agent-session-loop-a-1",
					),
				),
			)
			.toBeTruthy();
		const refreshInvocations = await page.evaluate(
			() => window.__RELEASH_BACKEND__?.invocations ?? [],
		);
		expect(
			refreshInvocations.filter(
				(entry) => entry.cmd === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(0);
		await expect(firstRow).toHaveAttribute("aria-current", "page");

		await page.evaluate(
			({ worktreePath, workflowSummary, firstOccurrence }) => {
				const internals = window.__RELEASH_BACKEND__;
				if (!internals) throw new Error("Tauri mock not initialized");
				const completedFirst = {
					...firstOccurrence,
					status: "idle",
					updatedAt: 2000,
				};
				const secondOccurrence = {
					...firstOccurrence,
					id: "occurrence-a-2",
					status: "active",
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
					archivedSessions: [],
					preferredNodeId: "occurrence-a-2",
				});
				internals.setMockResponse("get_workspace_node_detail", {
					processPresence: "unknown",
					statusClassification: "idle",
					id: "occurrence-a-1",
					title: "Loop step",
					status: "completed",
					capabilities: {
						canRename: false,
						canApprove: false,
						canRetry: false, canResumeSession: false,
					},
					updatedAt: 2000,
					submitReceived: false,
					stopReceived: false,
					hasArtifact: false,
					content: {
						kind: "session",
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
			name: "Loop step, idle",
			exact: true,
		});
		const secondRow = page.getByRole("button", {
			name: "Loop step, active",
			exact: true,
		});
		await expect(completedFirstRow).toHaveAttribute("aria-current", "page");
		await expect(secondRow).not.toHaveAttribute("aria-current");
		const updateInvocations = await page.evaluate(
			() => window.__RELEASH_BACKEND__?.invocations ?? [],
		);
		expect(
			updateInvocations.filter(
				(entry) => entry.cmd === "get_workspace_tree_selection_reconciliation",
			),
		).toHaveLength(0);

		const secondSession = agentSession("agent-session-loop-a-2", worktreePath);
		await page.evaluate(
			({ worktreePath, secondSession }) => {
				const internals = window.__RELEASH_BACKEND__;
				if (!internals) throw new Error("Tauri mock not initialized");
				internals.setMockResponse("get_workspace_node_detail", {
					processPresence: "unknown",
					statusClassification: "active",
					id: "occurrence-a-2",
					title: "Loop step",
					status: "running",
					capabilities: {
						canRename: false,
						canApprove: false,
						canRetry: false, canResumeSession: false,
					},
					updatedAt: 3000,
					submitReceived: false,
					stopReceived: false,
					hasArtifact: false,
					content: {
						kind: "session",
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
					window.__RELEASH_BACKEND__?.invocations.find(
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
		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).toBeVisible();

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
		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).toBeVisible();

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
		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).toBeVisible();

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

test("worktree作成完了のfrontend通知で一覧を再取得し新しいworktreeを表示する", async ({ page }) => {
	const created = { ...kanbanBranches[0], name: "feat/created", worktree_path: "/test/repo-worktrees/created" };
	await setupTauriMock(page, buildMockConfig({
		list_branches_with_status: kanbanBranches,
		create_worktree: { name: "created", path: created.worktree_path, branch: created.name, is_main: false, is_locked: false, dirty_count: 0, base_branch: "main" },
	}));
	await waitForApp(page);
	await page.evaluate(({ created, branches }) => {
        window.addEventListener("branch-list-refresh", () => performance.mark("worktree-created-notification"));
		const backend = window.__RELEASH_BACKEND__!;
		const execute = backend.execute;
		backend.execute = async (command, args) => {
			const result = await execute(command, args);
			if (command === "create_worktree") backend.setMockResponse("list_branches_with_status", [...branches, created]);
			return result;
		};
	}, { created, branches: kanbanBranches });
	await page.getByRole("button", { name: "Add Worktree", exact: true }).click();
	await page.getByLabel("Branch name", { exact: true }).fill(created.name);
	await page.getByRole("button", { name: "Create", exact: true }).click();
	await expect(page.getByTestId(`worktree-item-${created.name}`)).toBeVisible();
	const calls = await page.evaluate(() => window.__RELEASH_BACKEND__!.invocations.map(({ cmd }) => cmd));
	const creation = calls.indexOf("create_worktree");
	expect(creation).toBeGreaterThanOrEqual(0);
	expect(calls.slice(creation + 1)).toContain("list_branches_with_status_snapshot");
    expect(await page.evaluate(() => performance.getEntriesByName("worktree-created-notification").length)).toBe(1);
	const ipc = await page.evaluate(() => window.__TAURI_INTERNALS__!.ipcInvocations);
	expect(ipc.some(({ cmd }) => cmd === "plugin:event|emit" || cmd === "create_worktree")).toBe(false);
});


test("Archiveの画面確認後は期限を超えても共通操作の完了を一度だけ待つ", async ({page}) => {
    const branch = kanbanBranches.find(branch => branch.name === "feat/wip")!;
    await setupTauriMock(page, buildMockConfig({
        list_branches_with_status: [branch],
        list_workspace_worktree_nodes: {nodes: [{kind: "node", processPresence: "unknown", id: "archive-session", title: "Late Archive", status: "idle", contentKind: "session", capabilities: {canRename: false, canApprove: false, canRetry: false, canResumeSession: false}, workflowCapabilities: {canAbort: true, canArchive: true}, sessionCapabilities: {sessionRef: "archive-session", canArchive: true, canDelete: false}, pastAttempts: [], pastAttemptsCollapsed: false, updatedAt: 1}], archivedSessions: [], preferredNodeId: null},
        archive_workspace_workflow_execution: null,
    }));
    await waitForApp(page);
    await page.clock.install();
    await page.evaluate(() => {
        const backend = window.__RELEASH_BACKEND__!;
        const execute = backend.execute;
        backend.execute = async (command, args) => {
            const result = await execute(command, args);
            if (command === "archive_workspace_workflow_execution") {
                await new Promise<void>(resolve => window.addEventListener("finish-archive", () => resolve(), {once: true}));
                backend.setMockResponse("list_workspace_worktree_nodes", {nodes: [], archivedSessions: [], preferredNodeId: null});
            }
            return result;
        };
    });
    await page.getByRole("button", {name: "Late Archive, idle", exact: true}).hover();
    await page.getByRole("button", {name: "Archive Late Archive", exact: true}).click();
    await expect(page.getByText("Archiving will Abort this execution and stop its processes.")).toBeVisible();
    expect(await page.evaluate(() => window.__RELEASH_BACKEND__!.invocations.filter(({cmd}) => cmd === "archive_workspace_workflow_execution").length)).toBe(0);
    await page.getByRole("button", {name: "Abort and Archive", exact: true}).click();
    await expect.poll(() => page.evaluate(() => window.__RELEASH_BACKEND__!.invocations.filter(({cmd}) => cmd === "archive_workspace_workflow_execution").length)).toBe(1);
    await page.clock.fastForward(31_000);
    await expect(page.getByText(/操作結果を確認できません|再接続|未送信/)).toHaveCount(0);
    await expect(page.getByRole("button", {name: "元の操作の結果を確認", exact: true})).toHaveCount(0);
    await page.evaluate(() => window.dispatchEvent(new Event("finish-archive")));
    await expect(page.getByRole("button", {name: "Late Archive, idle", exact: true})).toHaveCount(0);
    await expect(page.getByText(/操作結果を確認できません/)).toHaveCount(0);
    expect(await page.evaluate(() => window.__RELEASH_BACKEND__!.invocations.filter(({cmd}) => cmd === "archive_workspace_workflow_execution").length)).toBe(1);
});
