import { invoke } from "@tauri-apps/api/core";
import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { firstState } from "./lib/client";
import { installPerformanceCollector } from "./test/performance/performanceCollector";

vi.mock("@wdio/tauri-plugin", () => ({}));
vi.mock("./test/performance/performanceCollector", () => ({
	installPerformanceCollector: vi.fn(),
}));
vi.mock("./test/performance/TerminalPerformanceScreen", () => ({
	TerminalPerformanceScreen: () => null,
}));
vi.mock("./App", () => ({ default: () => null }));
vi.mock("./hooks/useShikiHighlighter", () => ({ preloadHighlighter: vi.fn() }));
vi.mock("./lib/telemetry", () => ({
	installFrontendErrorHandlers: vi.fn(),
	reportFrontendError: vi.fn(),
}));
vi.mock("./components/ErrorBoundary", () => ({
	FrontendErrorBoundary: () => null,
}));
const { render } = vi.hoisted(() => ({ render: vi.fn() }));
vi.mock("react-dom/client", () => ({
	default: { createRoot: vi.fn(() => ({ render })) },
}));

beforeEach(() => {
	vi.resetModules();
	vi.clearAllMocks();
	vi.stubEnv("MODE", "performance");
});

afterEach(() => vi.unstubAllEnvs());

it.each([false, true])(
	"performance bootstrapは実アプリモードを購読の最初の値から取る（実アプリ: %s）",
	async (realAppMode) => {
		vi.mocked(firstState).mockResolvedValue({
			realAppMode,
			terminal: {
				disableOutputFlowControl: false,
				disableTerminalJournal: false,
				disableRendererWriteSerialization: false,
				disableWebglRenderer: false,
			},
		});

		await import("./main");
		await waitFor(() => expect(render).toHaveBeenCalledOnce());

		expect(firstState).toHaveBeenCalledExactlyOnceWith("performance-switches");
		expect(invoke).not.toHaveBeenCalled();
		expect(installPerformanceCollector).toHaveBeenCalledTimes(
			realAppMode ? 1 : 0,
		);
	},
);
