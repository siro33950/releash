import {
	create,
	fromBinary,
	fromJson,
	toBinary,
	toJson,
} from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
	CommandResultSchema,
	EnvelopeSchema,
	StreamSchema,
	TerminalEventSchema,
	WorkflowValueSchema,
} from "@/generated/client_pb";
import { buildWorkspaceState } from "@/types/workspace-state";
import { clientJson } from "./clientJson";
import {
	ClientStreamDecoder,
	decodeClientValue,
	encodeClientAck,
	encodeClientCommand,
} from "./clientProtocol";

describe("clientProtocol", () => {
	it.each([
		"listWorkspaceWorktreeNodes",
		"getWorkspaceTreeSelectionReconciliation",
	])("%s はchildrenのない過去試行もnodeとして復号する", (command) => {
		const past = {
			kind: "node",
			id: "past",
			title: "Past",
			status: "active",
			contentKind: "command",
			capabilities: {
				canRename: false,
				canApprove: false,
				canRetry: true,
			},
			pastAttempts: [],
			pastAttemptsCollapsed: true,
			updatedAt: 1,
		};
		const snapshot = {
			nodes: [{ ...past, id: "current", pastAttempts: [past] }],
			archivedSessions: [],
		};
		const expected =
			command === "listWorkspaceWorktreeNodes"
				? snapshot
				: { snapshot, reconciliation: { selectionInSnapshot: true } };
		const field = CommandResultSchema.fields.find(
			(field) => field.localName === command,
		);
		if (!field?.message) throw new Error("Missing result schema");
		const value = fromJson(CommandResultSchema, {
			[field.jsonName]: clientJson(field.message, expected, true),
		});
		const decoded = fromBinary(
			CommandResultSchema,
			toBinary(CommandResultSchema, value),
		);
		expect(decodeClientValue(decoded)).toEqual(expected);
	});
	it("UIが生成したworkspace stateを省略なく保存要求として送れる", () => {
		const state = buildWorkspaceState(
			{
				tabs: [{ path: "/repo/main.rs", name: "main.rs" }],
				activeEditorPath: "/repo/main.rs",
				activeView: "diff",
				rightBottomCollapsed: false,
				reviewCollapsed: true,
				diffOnlyMode: true,
				selectedDiffFile: "main.rs",
			},
			"editor",
			true,
			false,
		);
		const { body } = fromBinary(
			EnvelopeSchema,
			encodeClientCommand("save", "save_workspace_state", {
				worktreeName: "repo",
				state,
			}),
		);
		expect(body).toMatchObject({
			case: "request",
			value: {
				command: {
					case: "saveWorkspaceState",
					value: {
						state: {
							layout: {
								reviewCollapsed: true,
								diffOnlyMode: true,
								selectedDiffFile: "main.rs",
							},
						},
					},
				},
			},
		});
	});
	it("proto commandとackを生成し未定義commandを拒否する", () => {
		const { body } = fromBinary(
			EnvelopeSchema,
			encodeClientCommand("id", "write_terminal_surface", {
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "a",
				sequence: 0,
				data: "日本語",
				optional: undefined,
			}),
		);
		expect(body.case).toBe("request");
		if (body.case !== "request") throw new Error("request");
		expect(body.value.requestId).toBe("id");
		expect(body.value.command).toMatchObject({
			case: "writeTerminalSurface",
			value: { attachmentId: "a", sequence: 0n, data: "日本語" },
		});
		expect(() =>
			encodeClientCommand("id", "get_terminal_stream_endpoint"),
		).toThrow();
		expect(
			fromBinary(EnvelopeSchema, encodeClientAck("a", 42)).body,
		).toMatchObject({
			case: "ack",
			value: { attachmentId: "a", sequence: 42n },
		});
	});
	it("UTF8をまたぐ分割をattachmentごとに順序付けて完全復元する", () => {
		const a = new ClientStreamDecoder();
		const b = new ClientStreamDecoder();
		const data = "日本語🙂".repeat(100000);
		const bytes = toBinary(
			TerminalEventSchema,
			create(TerminalEventSchema, {
				item: {
					case: "output",
					value: { sessionKey: "workspace:a", data, sequence: 900n },
				},
			}),
		);
		const part = (
			attachmentId: string,
			sequence: bigint,
			data: Uint8Array,
			end: boolean,
		) => create(StreamSchema, { attachmentId, sequence, data, end });
		expect(a.decode(part("a", 1n, bytes.slice(0, 17), false))).toBeNull();
		expect(b.decode(part("b", 1n, bytes, true))).toEqual({
			type: "output",
			session_key: "workspace:a",
			data,
			sequence: 900,
		});
		expect(a.decode(part("a", 2n, bytes.slice(17), true))).toEqual({
			type: "output",
			session_key: "workspace:a",
			data,
			sequence: 900,
		});
		expect(() => a.decode(part("a", 4n, bytes, true))).toThrow("Out-of-order");
	});
	it("snapshotとresizeとexitと入力不可を既存描画型へ変換する", () => {
		const cases = [
			[
				{
					snapshot: {
						sessionKey: "a",
						replay: "screen",
						sequence: "41",
						cols: 80,
						rows: 24,
						isExited: true,
						exitCode: 3,
					},
				},
				{
					type: "snapshot",
					surface: {
						session_key: "a",
						terminal_surface: {
							replay: "screen",
							sequence: 41,
							cols: 80,
							rows: 24,
						},
						is_exited: true,
						exit_code: 3,
					},
				},
			],
			[
				{ resize: { sessionKey: "a", sequence: "42", cols: 90, rows: 30 } },
				{ type: "resize", session_key: "a", sequence: 42, cols: 90, rows: 30 },
			],
			[
				{ exit: { sessionKey: "a", sequence: "43" } },
				{ type: "exit", session_key: "a", sequence: 43, exit_code: null },
			],
			[
				{ inputUnavailable: { sessionKey: "a", message: "closed" } },
				{ type: "input_unavailable", session_key: "a", message: "closed" },
			],
		];
		for (const [input, expected] of cases) {
			const bytes = toBinary(
				TerminalEventSchema,
				fromJson(TerminalEventSchema, input as never),
			);
			expect(
				new ClientStreamDecoder().decode(
					create(StreamSchema, {
						attachmentId: "a",
						sequence: 1n,
						data: bytes,
						end: true,
					}),
				),
			).toEqual(expected);
		}
		expect(() =>
			new ClientStreamDecoder().decode(
				create(StreamSchema, { sequence: 1n, end: true }),
			),
		).toThrow("Missing terminal event");
	});
});

it("必須引数・整数範囲・tag付きownerをproto契約で検証する", () => {
	expect(() => encodeClientCommand("id", "get_current_branch", {})).toThrow(
		"Missing",
	);
	expect(() =>
		encodeClientCommand("id", "ack_terminal_surface_output", {
			attachmentId: "a",
			sequence: 0.5,
		}),
	).toThrow("integer");
	expect(() =>
		encodeClientCommand("id", "ack_terminal_surface_output", {
			attachmentId: "a",
			sequence: -1,
		}),
	).toThrow();
	const { body } = fromBinary(
		EnvelopeSchema,
		encodeClientCommand("id", "attach_terminal_surface", {
			attachmentId: "a",
			owner: { kind: "session", workspacePath: "/日本語", sessionId: "s" },
			recovery: false,
		}),
	);
	expect(body).toMatchObject({
		case: "request",
		value: {
			command: {
				case: "attachTerminalSurface",
				value: {
					recovery: false,
					owner: {
						variant: {
							case: "session",
							value: { workspacePath: "/日本語", sessionId: "s" },
						},
					},
				},
			},
		},
	});
});

it("動的workflow値の型をproto往復で保持する", () => {
	const value = { items: [null, true, -3, 0.25, "日本語", { nested: [] }] };
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
	).toEqual(value);
});
