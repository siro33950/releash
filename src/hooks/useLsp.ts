import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
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
	status: "idle" | "starting" | "running" | "error";
	error: string | null;
}

interface UseLspReturn {
	sessionId: number | null;
	status: LspState["status"];
	error: string | null;
	transport: TauriTransport | null;
}

/**
 * Hook that manages the lifecycle of an LSP server for a given worktree and language.
 *
 * - Detects the appropriate server via backend
 * - Spawns the server process
 * - Creates a TauriTransport for Monaco LSP client
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
	});
	const transportRef = useRef<TauriTransport | null>(null);
	const startingRef = useRef(false);

	useEffect(() => {
		if (!rootPath || !language) {
			return;
		}

		let cancelled = false;

		async function start() {
			if (startingRef.current) return;
			startingRef.current = true;
			setState({ sessionId: null, status: "starting", error: null });

			try {
				// Detect server from config / PATH
				const config = await invoke<LspServerConfig | null>(
					"detect_lsp_server",
					{ language },
				);

				if (cancelled) return;

				if (!config) {
					setState({
						sessionId: null,
						status: "idle",
						error: null,
					});
					startingRef.current = false;
					return;
				}

				// rootPath and language are guaranteed non-null by the guard at the top of the effect
				const transport = await createTauriTransport(
					rootPath as string,
					language as string,
					config.command,
					config.args,
				);

				if (cancelled) {
					await invoke("shutdown_lsp", {
						sessionId: transport.sessionId,
					});
					transport.dispose();
					startingRef.current = false;
					return;
				}

				transportRef.current = transport;
				setState({
					sessionId: transport.sessionId,
					status: "running",
					error: null,
				});
			} catch (e) {
				if (!cancelled) {
					const msg = e instanceof Error ? e.message : String(e);
					setState({ sessionId: null, status: "error", error: msg });
				}
			} finally {
				startingRef.current = false;
			}
		}

		start();

		return () => {
			cancelled = true;
			const transport = transportRef.current;
			if (transport) {
				transportRef.current = null;
				invoke("shutdown_lsp", { sessionId: transport.sessionId }).catch(
					() => {},
				);
				transport.dispose();
			}
			setState({ sessionId: null, status: "idle", error: null });
		};
	}, [rootPath, language]);

	// Listen for LSP error events from backend
	useEffect(() => {
		const unlisten = listen<{ session_id: number; error: string }>(
			"lsp-error",
			(event) => {
				if (
					transportRef.current &&
					event.payload.session_id === transportRef.current.sessionId
				) {
					setState((prev) => ({
						...prev,
						status: "error",
						error: event.payload.error,
					}));
					transportRef.current?.dispose();
					transportRef.current = null;
				}
			},
		);

		return () => {
			unlisten.then((fn) => fn());
		};
	}, []);

	return {
		sessionId: state.sessionId,
		status: state.status,
		error: state.error,
		transport: transportRef.current,
	};
}
