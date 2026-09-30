import { Channel, invoke } from "@tauri-apps/api/core";
import {
	createContext,
	type ReactNode,
	useContext,
	useEffect,
	useState,
} from "react";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "@/components/ui/dialog";
import { getConnectionState, onConnectionStateChange } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

interface DaemonStatus {
	phase: string;
	stage: string | null;
	reason: string | null;
	retries: number;
	retryAvailable: boolean;
}

const DaemonReadyContext = createContext(true);
export const useDaemonReady = () => useContext(DaemonReadyContext);

export function DaemonBoundary({ children }: { children: ReactNode }) {
	const [status, setStatus] = useState<DaemonStatus | null>(null);
	const [connection, setConnection] = useState(getConnectionState);
	const ready = status?.phase === "ready";
	const [shown, setShown] = useState(false);
	const [connected, setConnected] = useState(
		() => getConnectionState() === "READY",
	);
	const overlay =
		(!shown && !ready) ||
		["failed", "stopping", "installing", "stopped"].includes(
			status?.phase ?? "",
		);
	const [error, setError] = useState<string | null>(null);
	const [clientError, setClientError] = useState<string | null>(null);
	useEffect(() => {
		let active = true;
		let subscribed = false;
		const id = crypto.randomUUID();
		const stop = () => {
			void invoke("stop_daemon_status_subscription", { id }).catch((error) =>
				console.error("Failed to stop daemon status subscription", error),
			);
		};
		const channel = new Channel<DaemonStatus>();
		channel.onmessage = (next) => {
			if (active) {
				setStatus(next);
				if (next.phase === "ready") setShown(true);
			}
		};
		void invoke("subscribe_daemon_status", { id, channel })
			.then(() => {
				subscribed = true;
				if (!active) stop();
			})
			.catch((error) => {
				if (active) setError(getErrorMessage(error));
			});
		const release = onConnectionStateChange(() => {
			const next = getConnectionState();
			setConnection(next);
			if (next === "READY") setConnected(true);
		});
		const onClientError = (event: Event) =>
			setClientError((event as CustomEvent<string>).detail);
		window.addEventListener("releash-client-error", onClientError);
		return () => {
			active = false;
			if (subscribed) stop();
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
			{shown && (
				<DaemonReadyContext.Provider value={ready}>
					<div className="contents" inert={overlay} aria-hidden={overlay}>
						{children}
					</div>
				</DaemonReadyContext.Provider>
			)}
			{connected && connection !== "READY" && (
				<div
					role="status"
					data-client-connection={connection}
					className="fixed top-2 right-4 z-[110] rounded border bg-background px-3 py-2 text-sm shadow"
				>
					再接続中
				</div>
			)}
			{overlay && (
				<Dialog open>
					<DialogContent
						showCloseButton={false}
						className="inset-0 z-[100] flex w-full max-w-none translate-x-0 translate-y-0 items-center justify-center rounded-none border-0 bg-background/95 p-6 sm:max-w-none"
						onEscapeKeyDown={(event) => event.preventDefault()}
						onInteractOutside={(event) => event.preventDefault()}
					>
						<section
							aria-label="Daemon status"
							className="w-full max-w-xl space-y-4 rounded-lg border bg-card p-6"
						>
							<DialogTitle asChild>
								<h1 className="text-lg font-semibold">
									{status?.stage
										? `Releash: ${status.stage}`
										: status?.phase === "installing"
											? "Installing update…"
											: status?.phase === "stopping" ||
													status?.phase === "stopped"
												? "Stopping Releash…"
												: "Starting Releash…"}
								</h1>
							</DialogTitle>
							<DialogDescription asChild>
								<p role="status" data-client-connection={connection}>
									{status?.reason ?? "Waiting for the daemon connection."}
								</p>
							</DialogDescription>
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
					</DialogContent>
				</Dialog>
			)}
		</>
	);
}
