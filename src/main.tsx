import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { FrontendErrorBoundary } from "./components/ErrorBoundary";
import { preloadHighlighter } from "./hooks/useShikiHighlighter";
import "./index.css";
import {
	installFrontendErrorHandlers,
	reportFrontendError,
} from "./lib/telemetry";

async function bootstrap() {
	preloadHighlighter();
	installFrontendErrorHandlers();

	ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
		<React.StrictMode>
			<FrontendErrorBoundary>
				<App />
			</FrontendErrorBoundary>
		</React.StrictMode>,
	);
}

bootstrap().catch((err) => {
	reportFrontendError(err, "bootstrap_error");
	console.error("Failed to bootstrap application:", err);
});
