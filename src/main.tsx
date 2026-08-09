import React from "react";
import ReactDOM from "react-dom/client";
import { FrontendErrorBoundary } from "./components/ErrorBoundary";
import "./index.css";
import {
	installFrontendErrorHandlers,
	reportFrontendError,
} from "./lib/telemetry";

async function loadRealApp(): Promise<React.ReactNode> {
	const [{ default: App }, { preloadHighlighter }] = await Promise.all([
		import("./App"),
		import("./hooks/useShikiHighlighter"),
	]);
	preloadHighlighter();
	return <App />;
}

async function bootstrap() {
	let root: React.ReactNode;
	if (import.meta.env.MODE === "performance") {
		await import("@wdio/tauri-plugin");
		const [{ invoke }, { installPerformanceCollector }] = await Promise.all([
			import("@tauri-apps/api/core"),
			import("./test/performance/performanceCollector"),
		]);
		const realAppMode = await Promise.resolve()
			.then(() => invoke<boolean>("get_performance_real_app_mode"))
			.catch(() => false);
		if (realAppMode) {
			installPerformanceCollector();
			root = await loadRealApp();
		} else {
			const { TerminalPerformanceScreen } = await import(
				"./test/performance/TerminalPerformanceScreen"
			);
			root = <TerminalPerformanceScreen />;
		}
	} else {
		root = await loadRealApp();
	}
	installFrontendErrorHandlers();

	ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
		<React.StrictMode>
			<FrontendErrorBoundary>{root}</FrontendErrorBoundary>
		</React.StrictMode>,
	);
}

bootstrap().catch((err) => {
	reportFrontendError(err, "bootstrap_error");
	console.error("Failed to bootstrap application:", err);
});
