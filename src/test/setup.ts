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

vi.mock("@tauri-apps/plugin-fs", () => ({
	readDir: vi.fn().mockResolvedValue([]),
	readTextFile: vi.fn().mockResolvedValue(""),
	readFile: vi.fn().mockResolvedValue(new Uint8Array()),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
	open: vi.fn().mockResolvedValue(null),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
	check: vi.fn().mockResolvedValue(null),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
	relaunch: vi.fn().mockResolvedValue(undefined),
}));
