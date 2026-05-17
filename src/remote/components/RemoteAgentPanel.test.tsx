import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WsMessage } from "@/types/protocol";
import { RemoteAgentPanel } from "./RemoteAgentPanel";

function renderPanel(
	overrides: Partial<ComponentProps<typeof RemoteAgentPanel>> = {},
) {
	let handler: ((msg: WsMessage) => void) | null = null;
	const send = vi.fn();
	const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
		handler = cb;
		return vi.fn();
	});
	const props: ComponentProps<typeof RemoteAgentPanel> = {
		selectedWorktree: "/repo/worktree",
		backends: [
			{ id: "claude", name: "Claude", available: true, available_models: [] },
			{ id: "codex", name: "Codex", available: true, available_models: [] },
		],
		selectedBackendId: "codex",
		backendLoading: false,
		status: "connected",
		send,
		subscribe,
		onBackendChange: vi.fn(),
		onRefreshBackends: vi.fn(),
		...overrides,
	};

	render(<RemoteAgentPanel {...props} />);

	return {
		send,
		emit: (msg: WsMessage) => {
			act(() => {
				handler?.(msg);
			});
		},
	};
}

describe("RemoteAgentPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("sends agent_session_start_request with selected backend and worktree", async () => {
		const user = userEvent.setup();
		const { send } = renderPanel();

		await user.click(screen.getByText("Start Session"));

		expect(send).toHaveBeenCalledWith({
			type: "agent_session_start_request",
			payload: {
				worktree_path: "/repo/worktree",
				backend_id: "codex",
				permission_mode: "edit",
			},
		});
	});

	it("includes the selected abstract permission_mode in agent_session_start_request", async () => {
		const user = userEvent.setup();
		const { send } = renderPanel();

		const select = screen.getByTestId(
			"remote-permission-mode-select",
		) as HTMLSelectElement;
		await user.selectOptions(select, "readonly");
		await user.click(screen.getByText("Start Session"));

		expect(send).toHaveBeenLastCalledWith({
			type: "agent_session_start_request",
			payload: {
				worktree_path: "/repo/worktree",
				backend_id: "codex",
				permission_mode: "readonly",
			},
		});
	});

	it("shows ready state after successful start", () => {
		const { emit } = renderPanel();

		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		expect(screen.getByText("Session ready")).toBeInTheDocument();
		expect(screen.getByText("codex / session-1")).toBeInTheDocument();
	});

	it("sends a message, creates local human text, and renders streamed text deltas", async () => {
		const user = userEvent.setup();
		const { send, emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		await user.type(screen.getByPlaceholderText("Message"), "Hello");
		await user.click(screen.getByLabelText("Send message"));

		expect(screen.getByText("Hello")).toBeInTheDocument();
		expect(send).toHaveBeenLastCalledWith({
			type: "agent_message_request",
			payload: {
				session_id: "session-1",
				worktree_path: "/repo/worktree",
				content: "Hello",
				permission_mode: "edit",
				backend_id: null,
			},
		});

		emit({
			type: "agent_message_response",
			payload: {
				success: true,
				session_id: "session-1",
				agent_message_id: "agent-1",
				backend_id: "codex",
			},
		});
		// Rust emits the cumulative streaming_parts on every flush — Remote
		// replaces the message's parts on receipt.
		emit({
			type: "agent_stream_sync",
			payload: {
				session_id: "session-1",
				message_id: "agent-1",
				parts: [{ type: "text", content: "Hel" }],
			},
		});
		// First sync must surface in the DOM before the second arrives — long
		// streamed responses are observed incrementally, not only at the end.
		expect(screen.getByText("Hel")).toBeInTheDocument();

		emit({
			type: "agent_stream_sync",
			payload: {
				session_id: "session-1",
				message_id: "agent-1",
				parts: [{ type: "text", content: "Hello" }],
			},
		});

		// Cumulative replace: "Hel" is no longer rendered after the next sync.
		expect(screen.queryByText("Hel")).not.toBeInTheDocument();
		// "Hello" appears in both the echoed human input and the agent reply.
		expect(screen.getAllByText("Hello")).toHaveLength(2);
	});

	it("creates an agent message when stream sync arrives with an unknown message id", () => {
		const { emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		emit({
			type: "agent_stream_sync",
			payload: {
				session_id: "session-1",
				message_id: "pending-agent-1",
				parts: [{ type: "text", content: "Recovered stream" }],
			},
		});

		expect(screen.getByText("Recovered stream")).toBeInTheDocument();
	});

	it("ignores stream sync from another session", () => {
		const { emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		emit({
			type: "agent_stream_sync",
			payload: {
				session_id: "session-2",
				message_id: "agent-from-other-session",
				parts: [{ type: "text", content: "Other session output" }],
			},
		});

		expect(screen.queryByText("Other session output")).not.toBeInTheDocument();
	});

	it("locks backend selection after session start", async () => {
		const user = userEvent.setup();
		const onBackendChange = vi.fn();
		const { emit } = renderPanel({
			selectedBackendId: "codex",
			onBackendChange,
		});

		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		const select = screen.getByLabelText("Backend");
		expect(select).toBeDisabled();
		expect(select).toHaveValue("codex");
		await user.selectOptions(select, "claude");
		expect(onBackendChange).not.toHaveBeenCalled();
		expect(select).toHaveValue("codex");
	});

	it("clears running controls when agent_state_sync reports completion", () => {
		const { emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});
		emit({
			type: "agent_message_response",
			payload: {
				success: true,
				session_id: "session-1",
				agent_message_id: "agent-1",
				backend_id: "codex",
			},
		});
		expect(screen.getByLabelText("Interrupt agent")).toBeInTheDocument();

		emit({
			type: "agent_state_sync",
			payload: {
				worktree_path: "/repo/worktree",
				state: "done",
				exit_code: null,
				timestamp: 1000,
				session_id: "session-1",
			},
		});

		expect(screen.queryByLabelText("Interrupt agent")).not.toBeInTheDocument();
		expect(screen.getByLabelText("Send message")).toBeInTheDocument();
	});

	it("clears running controls when agent_state_sync reports an error", () => {
		const { emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});
		emit({
			type: "agent_message_response",
			payload: {
				success: true,
				session_id: "session-1",
				agent_message_id: "agent-1",
				backend_id: "codex",
			},
		});
		expect(screen.getByLabelText("Interrupt agent")).toBeInTheDocument();

		emit({
			type: "agent_state_sync",
			payload: {
				worktree_path: "/repo/worktree",
				state: "error",
				exit_code: 1,
				timestamp: 1000,
				session_id: "session-1",
			},
		});

		expect(screen.queryByLabelText("Interrupt agent")).not.toBeInTheDocument();
		expect(screen.getByLabelText("Send message")).toBeInTheDocument();
	});

	it("sends interrupt and model set commands for the active session", async () => {
		const user = userEvent.setup();
		const { send, emit } = renderPanel({
			backends: [
				{ id: "claude", name: "Claude", available: true, available_models: [] },
				{
					id: "codex",
					name: "Codex",
					available: true,
					available_models: [{ value: "gpt-5.4" }],
				},
			],
		});
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});
		emit({
			type: "agent_message_response",
			payload: {
				success: true,
				session_id: "session-1",
				agent_message_id: "agent-1",
				backend_id: "codex",
			},
		});

		await user.click(screen.getByLabelText("Interrupt agent"));
		await user.selectOptions(screen.getByLabelText("Model"), "gpt-5.4");
		await user.click(screen.getByText("Set"));

		expect(send).toHaveBeenCalledWith({
			type: "agent_interrupt_request",
			payload: { session_id: "session-1" },
		});
		expect(send).toHaveBeenCalledWith({
			type: "agent_model_set_request",
			payload: { session_id: "session-1", model_id: "gpt-5.4" },
		});
	});

	it("offers only the three abstract permission options without legacy vocabulary", () => {
		renderPanel();
		const select = screen.getByTestId(
			"remote-permission-mode-select",
		) as HTMLSelectElement;
		const labels = Array.from(select.options).map((o) => o.textContent);
		expect(labels).toEqual(["Read Only", "Edit", "Full"]);
		for (const legacy of [
			"acceptEdits",
			"bypassPermissions",
			"plan",
			"default",
		]) {
			expect(labels).not.toContain(legacy);
		}
	});

	it("presents registered model candidates and sends the selected value", async () => {
		const user = userEvent.setup();
		const { send, emit } = renderPanel({
			backends: [
				{ id: "claude", name: "Claude", available: true, available_models: [] },
				{
					id: "codex",
					name: "Codex",
					available: true,
					available_models: [{ value: "gpt-5.4" }],
				},
			],
		});
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		expect(screen.getByRole("option", { name: "gpt-5.4" })).toBeInTheDocument();
		expect(screen.queryByRole("option", { name: "Unset" })).toBeNull();
		await user.selectOptions(screen.getByLabelText("Model"), "gpt-5.4");
		await user.click(screen.getByText("Set"));

		expect(send).toHaveBeenCalledWith({
			type: "agent_model_set_request",
			payload: { session_id: "session-1", model_id: "gpt-5.4" },
		});
	});

	it("keeps the model selector visible with zero candidates when backend has no models", () => {
		const { emit } = renderPanel({
			backends: [
				{ id: "claude", name: "Claude", available: true, available_models: [] },
				{ id: "codex", name: "Codex", available: true, available_models: [] },
			],
		});
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		const modelSelect = screen.getByLabelText("Model");
		const options = within(modelSelect).queryAllByRole("option");
		expect(options).toHaveLength(0);
		expect(screen.queryByRole("option", { name: "Unset" })).toBeNull();
		expect(screen.queryByRole("option", { name: "gpt-5.4" })).toBeNull();
	});

	it("sends null model_id only through the clear model action", async () => {
		const user = userEvent.setup();
		const { send, emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		await user.click(screen.getByLabelText("Clear model"));

		expect(send).toHaveBeenCalledWith({
			type: "agent_model_set_request",
			payload: { session_id: "session-1", model_id: null },
		});
	});

	it("sends permission mode updates for an active remote session", async () => {
		const user = userEvent.setup();
		const { send, emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		await user.selectOptions(
			screen.getByTestId("remote-permission-mode-select"),
			"full",
		);

		expect(send).toHaveBeenLastCalledWith({
			type: "agent_permission_mode_set_request",
			payload: { session_id: "session-1", permission_mode: "full" },
		});
	});

	it("includes the selected abstract permission_mode in agent_message_request", async () => {
		const user = userEvent.setup();
		const { send, emit } = renderPanel();
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		const select = screen.getByTestId(
			"remote-permission-mode-select",
		) as HTMLSelectElement;
		await user.selectOptions(select, "readonly");
		await user.type(screen.getByPlaceholderText("Message"), "Hi");
		await user.click(screen.getByLabelText("Send message"));

		expect(send).toHaveBeenLastCalledWith({
			type: "agent_message_request",
			payload: {
				session_id: "session-1",
				worktree_path: "/repo/worktree",
				content: "Hi",
				permission_mode: "readonly",
				backend_id: null,
			},
		});
	});

	it("renders dangerous model identifiers as plain text on remote UI", async () => {
		// spec: メイン画面・リモート画面のいずれでも、表示時に副作用を起こしうる文字を
		// 含むモデル識別子は文字列としてのみ表示され、画面上で実行・解釈されない。
		const user = userEvent.setup();
		const dangerous = "<script>window.__pwned_remote=true</script>";
		const { send, emit } = renderPanel({
			backends: [
				{ id: "claude", name: "Claude", available: true, available_models: [] },
				{
					id: "codex",
					name: "Codex",
					available: true,
					available_models: [{ value: dangerous }],
				},
			],
		});
		emit({
			type: "agent_session_start_response",
			payload: {
				success: true,
				session_id: "session-1",
				backend_id: "codex",
			},
		});

		expect(screen.getByRole("option", { name: dangerous })).toBeInTheDocument();
		await user.selectOptions(screen.getByLabelText("Model"), dangerous);
		await user.click(screen.getByText("Set"));

		// 登録済み候補として提示された識別子はそのまま payload に乗る。
		expect(send).toHaveBeenCalledWith({
			type: "agent_model_set_request",
			payload: { session_id: "session-1", model_id: dangerous },
		});

		// option のテキストとして表示されるため、script 実行や onerror 属性の挿入は発生しない。
		expect(document.querySelectorAll("script").length).toBe(0);
		expect(document.querySelectorAll("[onerror]").length).toBe(0);
		expect(
			(window as unknown as { __pwned_remote?: boolean }).__pwned_remote,
		).toBeUndefined();
	});

	it("shows errors from failed agent responses", () => {
		const { emit } = renderPanel();

		emit({
			type: "agent_session_start_response",
			payload: { success: false, error: "bridge failed" },
		});
		expect(screen.getByText("bridge failed")).toBeInTheDocument();

		emit({
			type: "agent_message_response",
			payload: { success: false, error: "message failed" },
		});
		expect(screen.getByText("message failed")).toBeInTheDocument();

		emit({
			type: "agent_interrupt_response",
			payload: {
				success: false,
				session_id: "session-1",
				error: "interrupt failed",
			},
		});
		expect(screen.getByText("interrupt failed")).toBeInTheDocument();

		emit({
			type: "agent_model_set_response",
			payload: {
				success: false,
				session_id: "session-1",
				error: "model failed",
			},
		});
		expect(screen.getByText("model failed")).toBeInTheDocument();
	});
});
