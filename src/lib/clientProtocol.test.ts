import {
	create,
	fromBinary,
	fromJson,
	toBinary,
	toJson,
} from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
	AckTerminalSurfaceOutputRequestSchema,
	NodeExecutionStatusViewSchema,
	PushSchema,
	RefreshWorkspacesRequestSchema,
	TerminalEventSchema,
	UpdateCrashReportingRequestSchema,
	WorkflowValueSchema,
	WorkspaceListSnapshotDtoSchema,
} from "@/generated/client_pb";
import { clientJson } from "./clientJson";
import { decodeClientPush, decodeTerminalEvent } from "./clientProtocol";

describe("Connect message codecs", () => {
	it("生成されたpushを既存の表示用payloadへ戻す", () => {
		const push = create(PushSchema, {
			event: { case: "repoPathsChanged", value: { items: ["/repo"] } },
		});
		expect(decodeClientPush(push, "repo-paths-changed")).toEqual(["/repo"]);
		expect(() => decodeClientPush(push, "branch-list-sync")).toThrow(
			"Invalid push event",
		);
	});
	it("terminalのsnapshotと出力と終了を復号する", () => {
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "snapshot",
						value: {
							sessionKey: "terminal",
							replay: "screen",
							sequence: 42n,
							cols: 80,
							rows: 24,
						},
					},
				}),
			),
		).toEqual({
			type: "snapshot",
			surface: {
				session_key: "terminal",
				terminal_surface: {
					replay: "screen",
					sequence: 42,
					cols: 80,
					rows: 24,
				},
				is_exited: false,
				exit_code: null,
			},
		});
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "output",
						value: { sessionKey: "terminal", data: "next", sequence: 43n },
					},
				}),
			),
		).toEqual({
			type: "output",
			session_key: "terminal",
			data: "next",
			sequence: 43,
		});
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "exit",
						value: { sessionKey: "terminal", exitCode: 0, sequence: 44n },
					},
				}),
			),
		).toEqual({
			type: "exit",
			session_key: "terminal",
			exit_code: 0,
			sequence: 44,
		});
		expect(() => decodeTerminalEvent(create(TerminalEventSchema))).toThrow(
			"Missing terminal event",
		);
	});
	it("resizeと入力不可と終了済みsnapshotを変換する", () => {
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "resize",
						value: {
							sessionKey: "terminal",
							rows: 37,
							cols: 111,
							sequence: 9n,
						},
					},
				}),
			),
		).toEqual({
			type: "resize",
			session_key: "terminal",
			rows: 37,
			cols: 111,
			sequence: 9,
		});
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "inputUnavailable",
						value: { sessionKey: "terminal", message: "Input unavailable" },
					},
				}),
			),
		).toEqual({
			type: "input_unavailable",
			session_key: "terminal",
			message: "Input unavailable",
		});
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "snapshot",
						value: {
							sessionKey: "terminal",
							replay: "done",
							rows: 37,
							cols: 111,
							sequence: 10n,
							isExited: true,
							exitCode: 3,
						},
					},
				}),
			),
		).toEqual({
			type: "snapshot",
			surface: {
				session_key: "terminal",
				terminal_surface: { replay: "done", rows: 37, cols: 111, sequence: 10 },
				is_exited: true,
				exit_code: 3,
			},
		});
	});

	it("動的workflow値の型とネストをproto往復で保持する", () => {
		const value = {
			items: [null, true, false, -3, 0, 42, 0.25, "日本語", { nested: [] }],
			nested: { empty: {}, values: [null, { enabled: false }] },
		};
		const message = fromJson(
			WorkflowValueSchema,
			clientJson(WorkflowValueSchema, value, true),
		);
		const decoded = fromBinary(
			WorkflowValueSchema,
			toBinary(WorkflowValueSchema, message),
		);
		expect(
			clientJson(
				WorkflowValueSchema,
				toJson(WorkflowValueSchema, decoded),
				false,
			),
		).toStrictEqual(value);
	});

	it("入力境界の必須値と数値を検証する", () => {
		expect(() =>
			clientJson(UpdateCrashReportingRequestSchema, {}, true),
		).toThrow("Missing");
		expect(() =>
			clientJson(UpdateCrashReportingRequestSchema, { enabled: "yes" }, true),
		).toThrow();
		const json = clientJson(
			AckTerminalSurfaceOutputRequestSchema,
			{ attachmentId: "id", sequence: 42 },
			true,
		);
		expect(fromJson(AckTerminalSurfaceOutputRequestSchema, json).sequence).toBe(
			42n,
		);
	});
});

it("削除した未解決状態をwireとJSONのどちらからも受け入れない", () => {
	expect(() =>
		clientJson(NodeExecutionStatusViewSchema, { value: "unspecified" }, false),
	).toThrow("Invalid enum value");
	expect(() =>
		clientJson(NodeExecutionStatusViewSchema, "unresolved", true),
	).toThrow("Invalid enum value");
	expect(() => clientJson(NodeExecutionStatusViewSchema, "", true)).toThrow(
		"Invalid enum value",
	);
});

it.each([
	{ repositories: [] },
	{
		repositories: [
			{
				path: "/repo",
				status: { loaded: true, error: null },
				branches: [],
				worktrees: [],
			},
		],
	},
])("Workspacesの正常な空配列をprotobuf往復で保持する", ({ repositories }) => {
	const value = {
		generation: 1,
		status: { loaded: true, error: null },
		repositories,
	};
	const message = fromJson(
		WorkspaceListSnapshotDtoSchema,
		clientJson(WorkspaceListSnapshotDtoSchema, value, true),
	);
	const decoded = fromBinary(
		WorkspaceListSnapshotDtoSchema,
		toBinary(WorkspaceListSnapshotDtoSchema, message),
	);
	expect(
		clientJson(
			WorkspaceListSnapshotDtoSchema,
			toJson(WorkspaceListSnapshotDtoSchema, decoded),
			false,
		),
	).toEqual(value);
});

it.each([undefined, null, "/repo/worktree"])(
	"Workspacesの更新範囲をprotobufで保持する: %s",
	(worktreePath) => {
		const message = fromJson(
			RefreshWorkspacesRequestSchema,
			clientJson(
				RefreshWorkspacesRequestSchema,
				worktreePath === undefined ? {} : { worktreePath },
				true,
			),
		);
		const decoded = fromBinary(
			RefreshWorkspacesRequestSchema,
			toBinary(RefreshWorkspacesRequestSchema, message),
		);
		expect(decoded.worktreePath).toBe(worktreePath ?? undefined);
	},
);

it("Repositoryの更新範囲をprotobufで保持する", () => {
	const message = fromJson(
		RefreshWorkspacesRequestSchema,
		clientJson(RefreshWorkspacesRequestSchema, { repoPath: "/repo" }, true),
	);
	const decoded = fromBinary(
		RefreshWorkspacesRequestSchema,
		toBinary(RefreshWorkspacesRequestSchema, message),
	);
	expect(decoded.repoPath).toBe("/repo");
	expect(decoded.worktreePath).toBeUndefined();
});
