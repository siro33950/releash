import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

globalThis.ResizeObserver = class ResizeObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
};

Object.defineProperty(window, "matchMedia", {
	writable: true,
	value: vi.fn().mockImplementation((query: string) => ({
		matches: false,
		media: query,
		onchange: null,
		addListener: vi.fn(),
		removeListener: vi.fn(),
		addEventListener: vi.fn(),
		removeEventListener: vi.fn(),
		dispatchEvent: vi.fn(),
	})),
});

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(1),
	Channel: class {
		onmessage: ((message: unknown) => void) | undefined;
	},
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@xterm/xterm", () => {
	return {
		Terminal: class MockTerminal {
			loadAddon = vi.fn();
			open = vi.fn();
			write = vi.fn();
			refresh = vi.fn();
			onData = vi.fn().mockReturnValue({ dispose: vi.fn() });
			dispose = vi.fn();
			options: Record<string, unknown> = {};
			rows = 24;
			cols = 80;
		},
	};
});

vi.mock("@xterm/addon-fit", () => {
	return {
		FitAddon: class MockFitAddon {
			fit = vi.fn();
		},
	};
});

vi.mock("@xterm/addon-web-links", () => {
	return {
		WebLinksAddon: class MockWebLinksAddon {
			constructor(
				readonly handler?: (event: MouseEvent, uri: string) => void,
				readonly options?: Record<string, unknown>,
			) {}
		},
	};
});

vi.mock("@tauri-apps/plugin-dialog", () => ({
	open: vi.fn().mockResolvedValue(null),
}));

vi.mock("@/lib/client", async (importOriginal) => {
	const original = await importOriginal<typeof import("@/lib/client")>();
	return {
		...original,
		invokeClient: vi.fn().mockResolvedValue(1),
		firstState: vi.fn().mockRejectedValue(new Error("No state fixture")),
		subscribeState: vi.fn(
			(
				target: string | { kind: string },
				onValue: (value: unknown) => void,
			) => {
				const name = typeof target === "string" ? target : target.kind;
				if (
					[
						"repository-paths",
						"branches",
						"providers",
						"issues",
						"worktrees",
						"review-threads",
						"workflows",
						"facets",
					].includes(name)
				)
					onValue([]);
				else if (name === "notion-tasks")
					onValue({ page: { tasks: [], has_more: false } });
				else if (name === "notion-label-options") onValue({ options: [] });
				else if (name === "session-history")
					onValue({ items: [], hasMore: false });
				else if (name === "diagnostics")
					onValue({
						items: [],
						workflow_summaries: {},
						facet_summaries: {},
						facet_usage: {},
					});
				else if (
					[
						"workspace-state",
						"agent-session",
						"node-detail",
						"workflow",
						"workflow-source",
						"releash-base",
						"notion-config",
					].includes(name)
				)
					onValue(null);
				else if (name === "provider-hook-health") onValue([]);
				else if (name === "startup-outcome") onValue({ type: "ready" });
				else if (name === "desktop-settings")
					onValue({
						closeToTray: true,
						startMinimized: false,
						crashReporting: true,
						performanceTelemetry: true,
						autoLaunch: false,
					});
				else if (name === "workflow-config")
					onValue({ approval_auto_approve: false });
				else if (name === "external-editor")
					onValue({ selected: "", editors: [] });
				else if (name === "provider-availability") onValue({ providers: [] });
				return () => {};
			},
		),
	};
});
