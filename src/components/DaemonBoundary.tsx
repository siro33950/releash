import { Channel, invoke } from "@tauri-apps/api/core";
import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import {
	completeClientRestoration,
	getConnectionState,
	onConnectionStateChange,
} from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

interface DaemonStatus {
	phase: string;
	connectionGeneration: number;
	stage: string | null;
	reason: string | null;
	retries: number;
	retryAvailable: boolean;
}

const RestorationContext = createContext({
	ready: true,
	complete: async () => {},
	fail: async (_reason: string) => {},
});
export const useDesktopRestoration = () => useContext(RestorationContext);

export function DaemonBoundary({ children }: { children: ReactNode }) {
	const [status, setStatus] = useState<DaemonStatus | null>(null);
	const [connection, setConnection] = useState(getConnectionState);
	const ready = status?.phase === "ready";
	const restoring = status?.phase === "restoring";
	const generation = status?.connectionGeneration;
	const [error, setError] = useState<string | null>(null);
	const [clientError, setClientError] = useState<string | null>(null);
	useEffect(() => {
		let active = true;
		let subscribed = false;
		const id = crypto.randomUUID();
		const channel = new Channel<DaemonStatus>();
		channel.onmessage = (next) => {
			if (active) setStatus(next);
		};
		void invoke("subscribe_daemon_status", { id, channel })
			.then(() => {
				subscribed = true;
				if (!active) void invoke("stop_daemon_status_subscription", { id });
			})
			.catch((error) => {
				if (active) setError(getErrorMessage(error));
			});
		const release = onConnectionStateChange(() =>
			setConnection(getConnectionState()),
		);
		const onClientError = (event: Event) =>
			setClientError((event as CustomEvent<string>).detail);
		window.addEventListener("releash-client-error", onClientError);
		return () => {
			active = false;
			if (subscribed) void invoke("stop_daemon_status_subscription", { id });
			release();
			window.removeEventListener("releash-client-error", onClientError);
		};
	}, []);
	const action = async (command: "retry_daemon" | "quit_desktop") => {
		try {
			await invoke(command);
			setError(null);
		} catch (error) {
			setError(getErrorMessage(error));
		}
	};
	const fail = useCallback(
		async (reason: string) => {
			try {
				await invoke("fail_desktop_restoration", { generation, reason });
			} catch (error) {
				setError(getErrorMessage(error));
			}
		},
		[generation],
	);
	const complete = useCallback(async () => {
		if (generation === undefined) return;
		try {
			await completeClientRestoration(generation);
		} catch (error) {
			await fail(getErrorMessage(error));
		}
	}, [generation, fail]);
	const restoration = useMemo(
		() => ({ ready, complete, fail }),
		[ready, complete, fail],
	);
	return (
		<>
			{clientError && (
				<div
					role="alert"
					className="fixed bottom-4 right-4 z-[110] max-w-md rounded border bg-background p-3 text-sm text-destructive shadow-lg"
				>
					{clientError}
					<button
						type="button"
						className="ml-3"
						onClick={() => setClientError(null)}
						aria-label="Dismiss error"
					>
						×
					</button>
				</div>
			)}
			{(ready || restoring) && (
				<RestorationContext.Provider value={restoration}>
					<div
						key={status?.connectionGeneration}
						className="contents"
						inert={!ready}
						aria-hidden={!ready}
					>
						{children}
					</div>
				</RestorationContext.Provider>
			)}
			{!ready && (
				<div className="fixed inset-0 z-[100] flex items-center justify-center bg-background/95 p-6">
					<section
						aria-label="Daemon status"
						className="w-full max-w-xl space-y-4 rounded-lg border bg-card p-6"
					>
						<h1 className="text-lg font-semibold">
							{status?.stage
								? `Releash: ${status.stage}`
								: status?.phase === "installing"
									? "Installing update…"
									: status?.phase === "stopping" || status?.phase === "stopped"
										? "Stopping Releash…"
										: "Starting Releash…"}
						</h1>
						<p role="status" data-client-connection={connection}>
							{status?.reason ?? "Waiting for the daemon connection."}
						</p>
						{status?.phase === "backoff" && (
							<p>Restart attempt {status.retries} / 3</p>
						)}
						{error && <p role="alert">{error}</p>}
						<div className="flex gap-3">
							{status?.retryAvailable && (
								<button
									type="button"
									onClick={() => void action("retry_daemon")}
									className="rounded bg-primary px-4 py-2 text-primary-foreground"
								>
									Retry
								</button>
							)}
							<button
								type="button"
								onClick={() => void action("quit_desktop")}
								className="rounded border px-4 py-2"
							>
								Quit
							</button>
						</div>
					</section>
				</div>
			)}
		</>
	);
}
