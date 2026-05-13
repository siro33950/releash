import { act, render, screen } from "@testing-library/react";
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
			{ id: "claude", name: "Claude", available: true },
			{ id: "codex", name: "Codex", available: true },
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
				permission_mode: "acceptEdits",
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
		const { send, emit } = renderPanel();
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
		await user.type(screen.getByPlaceholderText("Model"), "gpt-5.4");
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
