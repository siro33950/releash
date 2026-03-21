import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue([]),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@/hooks/useAgentChat", () => ({
	useAgentChat: () => ({
		sessions: [],
		orderedSessions: [],
		closedSessions: [],
		activeSession: null,
		agentState: null,
		isStreaming: false,
		error: null,
		permissionMode: "acceptEdits",
		pendingPermission: null,
		sendMessage: vi.fn(),
		interrupt: vi.fn(),
		selectSession: vi.fn(),
		refreshSessions: vi.fn(),
		refreshClosedSessions: vi.fn(),
		closeSession: vi.fn(),
		restoreSession: vi.fn(),
		createNewSession: vi.fn(),
		reorderSessions: vi.fn(),
		setPermissionMode: vi.fn(),
		respondPermission: vi.fn(),
	}),
}));

vi.mock("@/hooks/useTerminal", () => ({
	useTerminal: () => ({
		terminalRef: { current: null },
		ptyId: null,
		isExited: false,
	}),
}));

vi.mock("@xterm/xterm", () => ({
	Terminal: vi.fn().mockImplementation(() => ({
		loadAddon: vi.fn(),
		open: vi.fn(),
		dispose: vi.fn(),
		onData: vi.fn(),
		onResize: vi.fn(),
		write: vi.fn(),
		rows: 24,
		cols: 80,
	})),
}));

vi.mock("@xterm/addon-fit", () => ({
	FitAddon: vi.fn().mockImplementation(() => ({
		fit: vi.fn(),
		dispose: vi.fn(),
	})),
}));

const { AgentTab } = await import("./AgentTab");

function renderAgentTab(props: { rootPath: string | null }) {
	return render(
		<TooltipProvider>
			<AgentTab {...props} />
		</TooltipProvider>,
	);
}

describe("AgentTab", () => {
	it("renders EmptyState when rootPath is null", () => {
		renderAgentTab({ rootPath: null });
		expect(screen.getByText("No worktree selected")).toBeDefined();
	});

	it("renders Chat view by default", () => {
		renderAgentTab({ rootPath: "/repo" });
		expect(screen.getByTestId("agent-chat-panel")).toBeDefined();
		expect(screen.getByLabelText("Chat view")).toBeDefined();
		expect(screen.getByLabelText("Terminal view")).toBeDefined();
	});

	it("switches to Terminal view when Terminal button is clicked", () => {
		renderAgentTab({ rootPath: "/repo" });
		const terminalButton = screen.getByLabelText("Terminal view");
		fireEvent.click(terminalButton);
		const chatContainer = screen.getByTestId("agent-chat-panel").parentElement;
		expect(chatContainer?.className).toContain("hidden");
	});

	it("switches back to Chat view when Chat button is clicked", () => {
		renderAgentTab({ rootPath: "/repo" });
		fireEvent.click(screen.getByLabelText("Terminal view"));
		fireEvent.click(screen.getByLabelText("Chat view"));
		const chatContainer = screen.getByTestId("agent-chat-panel").parentElement;
		expect(chatContainer?.className).not.toContain("hidden");
	});
});
