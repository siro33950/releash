import { invoke } from "@tauri-apps/api/core";
import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invokeClient } from "./lib/client";
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
	"performance bootstrapは実アプリモードをwsで取得する（実アプリ: %s）",
	async (realAppMode) => {
		vi.mocked(invokeClient).mockResolvedValue(realAppMode);

		await import("./main");
		await waitFor(() => expect(render).toHaveBeenCalledOnce());

		expect(invokeClient).toHaveBeenCalledExactlyOnceWith(
			"get_performance_real_app_mode",
		);
		expect(invoke).not.toHaveBeenCalled();
		expect(installPerformanceCollector).toHaveBeenCalledTimes(
			realAppMode ? 1 : 0,
		);
	},
);
