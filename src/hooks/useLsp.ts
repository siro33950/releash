import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	createTauriTransport,
	type TauriTransport,
} from "../lib/lsp/tauri-transport";

interface LspServerConfig {
	command: string;
	args: string[];
	enabled: boolean;
}

interface LspState {
	sessionId: number | null;
	status: "idle" | "starting" | "running" | "error" | "restarting" | "stopped";
	error: string | null;
	crashCount: number;
}

export interface UseLspReturn {
	sessionId: number | null;
	status: LspState["status"];
	error: string | null;
	transport: TauriTransport | null;
	crashCount: number;
	retryManually: () => void;
}

const MAX_RESTARTS = 4; // 5回目のクラッシュで停止
const WINDOW_MS = 3 * 60 * 1000; // 3分

/**
 * Hook that manages the lifecycle of an LSP server for a given worktree and language.
 *
 * - Detects the appropriate server via backend
 * - Spawns the server process
 * - Creates a TauriTransport for Monaco LSP client
 * - Auto-restarts on crash (sliding window: 5 crashes in 3min → stop)
 * - Shuts down on cleanup
 */
export function useLsp(
	rootPath: string | null,
	language: string | null,
): UseLspReturn {
	const [state, setState] = useState<LspState>({
		sessionId: null,
		status: "idle",
		error: null,
		crashCount: 0,
	});
	const transportRef = useRef<TauriTransport | null>(null);
	const startingRef = useRef(false);
	const unmountedRef = useRef(false);
	const crashTimestampsRef = useRef<number[]>([]);

	// Always keep the latest args accessible for restart
	const argsRef = useRef({ rootPath, language });
	argsRef.current = { rootPath, language };

	const startServer = useCallback(async (rp: string, lang: string) => {
		if (startingRef.current) return;
		startingRef.current = true;
		setState((prev) => ({
			...prev,
			sessionId: null,
			status: "starting",
			error: null,
		}));

		try {
			const config = await invoke<LspServerConfig | null>("detect_lsp_server", {
				language: lang,
			});

			if (unmountedRef.current) {
				startingRef.current = false;
				return;
			}

			if (!config) {
				setState((prev) => ({
					...prev,
					sessionId: null,
					status: "idle",
					error: null,
				}));
				startingRef.current = false;
				return;
			}

			const transport = await createTauriTransport(
				rp,
				lang,
				config.command,
				config.args,
			);

			if (unmountedRef.current) {
				await invoke("shutdown_lsp", {
					sessionId: transport.sessionId,
				});
				transport.dispose();
				startingRef.current = false;
				return;
			}

			// Verify args haven't changed during async startup
			if (
				argsRef.current.rootPath !== rp ||
				argsRef.current.language !== lang
			) {
				await invoke("shutdown_lsp", {
					sessionId: transport.sessionId,
				});
				transport.dispose();
				startingRef.current = false;
				return;
			}

			transportRef.current = transport;
			setState((prev) => ({
				...prev,
				sessionId: transport.sessionId,
				status: "running",
				error: null,
			}));
		} catch (e) {
			if (!unmountedRef.current) {
				const msg = e instanceof Error ? e.message : String(e);
				setState((prev) => ({
					...prev,
					sessionId: null,
					status: "error",
					error: msg,
				}));
			}
		} finally {
			startingRef.current = false;
		}
	}, []);

	// Initial startup effect
	useEffect(() => {
		unmountedRef.current = false;

		if (!rootPath || !language) {
			return;
		}

		// Reset crash tracking on rootPath/language change
		crashTimestampsRef.current = [];
		setState({ sessionId: null, status: "idle", error: null, crashCount: 0 });

		startServer(rootPath, language);

		return () => {
			unmountedRef.current = true;
			const transport = transportRef.current;
			if (transport) {
				transportRef.current = null;
				invoke("shutdown_lsp", { sessionId: transport.sessionId }).catch(
					() => {},
				);
				transport.dispose();
			}
			setState({ sessionId: null, status: "idle", error: null, crashCount: 0 });
		};
	}, [rootPath, language, startServer]);

	// Listen for LSP error events from backend — auto-restart with sliding window
	useEffect(() => {
		const unlisten = listen<{ session_id: number; error: string }>(
			"lsp-error",
			(event) => {
				if (
					!transportRef.current ||
					event.payload.session_id !== transportRef.current.sessionId
				) {
					return;
				}

				// Clean up crashed transport
				transportRef.current.dispose();
				transportRef.current = null;

				// Sliding window: prune timestamps older than WINDOW_MS
				const now = Date.now();
				const timestamps = crashTimestampsRef.current;
				while (timestamps.length > 0 && now - timestamps[0] > WINDOW_MS) {
					timestamps.shift();
				}
				timestamps.push(now);

				const { rootPath: rp, language: lang } = argsRef.current;

				if (timestamps.length > MAX_RESTARTS) {
					// Too many crashes — stop
					setState((prev) => ({
						...prev,
						sessionId: null,
						status: "stopped",
						error: event.payload.error,
						crashCount: timestamps.length,
					}));
				} else {
					// Auto-restart
					setState((prev) => ({
						...prev,
						sessionId: null,
						status: "restarting",
						error: event.payload.error,
						crashCount: timestamps.length,
					}));

					if (rp && lang) {
						startServer(rp, lang);
					}
				}
			},
		);

		return () => {
			unlisten.then((fn) => fn());
		};
	}, [startServer]);

	const retryManually = useCallback(() => {
		const { rootPath: rp, language: lang } = argsRef.current;
		if (!rp || !lang) return;

		// Reset crash tracking
		crashTimestampsRef.current = [];
		setState((prev) => ({ ...prev, crashCount: 0 }));

		startServer(rp, lang);
	}, [startServer]);

	return {
		sessionId: state.sessionId,
		status: state.status,
		error: state.error,
		transport: transportRef.current,
		crashCount: state.crashCount,
		retryManually,
	};
}
