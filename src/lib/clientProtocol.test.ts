import {
	create,
	fromBinary,
	fromJson,
	toBinary,
	toJson,
} from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
	NodeExecutionStatusViewSchema,
	RefreshWorkspacesRequestSchema,
	TerminalEventSchema,
	UpdateCrashReportingRequestSchema,
	WorkflowValueSchema,
	WorkspaceListSnapshotSchema,
	WriteTerminalSurfaceRequestSchema,
} from "@/generated/client_pb";
import { clientJson } from "./clientJson";
import { decodeTerminalEvent } from "./clientProtocol";

describe("Connect message codecs", () => {
	it("terminalのsnapshotと出力と終了を復号する", () => {
		expect(
			decodeTerminalEvent(
				create(TerminalEventSchema, {
					item: {
						case: "snapshot",
						value: {
							sessionKey: "terminal",
							processedReportUnits: 5000,
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
				processed_report_units: 5000,
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
	it("resizeと終了済みsnapshotを変換する", () => {
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
						case: "snapshot",
						value: {
							sessionKey: "terminal",
							processedReportUnits: 5000,
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
				processed_report_units: 5000,
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
			WriteTerminalSurfaceRequestSchema,
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "id",
				sequence: 42,
				data: "x",
				clientStartedAtUnixMs: null,
			},
			true,
		);
		expect(fromJson(WriteTerminalSurfaceRequestSchema, json).sequence).toBe(
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
				status: { loaded: true, error: null, state: "ready" },
				branches: [],
				worktrees: [],
			},
		],
	},
])("Workspacesの正常な空配列をprotobuf往復で保持する", ({ repositories }) => {
	const value = {
		status: { loaded: true, error: null, state: "ready" },
		repositories,
	};
	const message = fromJson(
		WorkspaceListSnapshotSchema,
		clientJson(WorkspaceListSnapshotSchema, value, true),
	);
	const decoded = fromBinary(
		WorkspaceListSnapshotSchema,
		toBinary(WorkspaceListSnapshotSchema, message),
	);
	expect(
		clientJson(
			WorkspaceListSnapshotSchema,
			toJson(WorkspaceListSnapshotSchema, decoded),
			false,
		),
	).toEqual(value);
});

it("Workspacesの更新要求は引数を持たずprotobufを往復する", () => {
	const message = fromJson(
		RefreshWorkspacesRequestSchema,
		clientJson(RefreshWorkspacesRequestSchema, {}, true),
	);
	const decoded = fromBinary(
		RefreshWorkspacesRequestSchema,
		toBinary(RefreshWorkspacesRequestSchema, message),
	);
	expect(
		clientJson(
			RefreshWorkspacesRequestSchema,
			toJson(RefreshWorkspacesRequestSchema, decoded),
			false,
		),
	).toEqual({});
});
