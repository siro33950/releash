import type * as Monaco from "monaco-editor";
import { useEffect, useRef, useState } from "react";
import type { TauriTransport } from "../lib/lsp/tauri-transport";

// Extract the transport type expected by MonacoLspClient's constructor.
// IMessageTransport is not exported from the monaco.lsp namespace directly,
// so we infer it from the constructor parameters.
type LspTransport = ConstructorParameters<typeof Monaco.lsp.MonacoLspClient>[0];

/**
 * Hook that connects a TauriTransport to Monaco's built-in LSP client.
 * When transport is available, creates a MonacoLspClient that automatically
 * registers all 21 LSP features (completion, hover, definition, diagnostics, etc.)
 * and manages textDocument synchronization.
 */
export function useLspMonaco(
	monaco: typeof Monaco | null,
	transport: TauriTransport | null,
): { connected: boolean } {
	const clientRef = useRef<Monaco.lsp.MonacoLspClient | null>(null);
	const [connected, setConnected] = useState(false);

	useEffect(() => {
		if (!monaco || !transport) {
			setConnected(false);
			return;
		}

		// monaco.lsp is available since Monaco 0.55.0
		if (!monaco.lsp?.MonacoLspClient) {
			console.warn("[useLspMonaco] monaco.lsp.MonacoLspClient not available");
			setConnected(false);
			return;
		}

		try {
			const client = new monaco.lsp.MonacoLspClient(
				transport as unknown as LspTransport,
			);
			clientRef.current = client;
			setConnected(true);
		} catch (e) {
			console.error("[useLspMonaco] Failed to create MonacoLspClient:", e);
			setConnected(false);
		}

		return () => {
			if (
				clientRef.current &&
				"dispose" in clientRef.current &&
				typeof clientRef.current.dispose === "function"
			) {
				clientRef.current.dispose();
			}
			clientRef.current = null;
			setConnected(false);
		};
	}, [monaco, transport]);

	return { connected };
}
