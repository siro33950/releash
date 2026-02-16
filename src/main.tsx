import { invoke } from "@tauri-apps/api/core";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { SentryErrorBoundary } from "./components/ErrorBoundary";
import "./index.css";
import { initSentry } from "./lib/sentry";

async function bootstrap() {
	let crashReportingEnabled = true;
	try {
		crashReportingEnabled = await invoke<boolean>(
			"get_crash_reporting_enabled",
		);
	} catch {
		// Rust側が応答しない場合はデフォルト(有効)で初期化
	}

	initSentry(crashReportingEnabled);

	ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
		<React.StrictMode>
			<SentryErrorBoundary>
				<App />
			</SentryErrorBoundary>
		</React.StrictMode>,
	);
}

bootstrap();
