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
	const root = await loadRealApp();
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
