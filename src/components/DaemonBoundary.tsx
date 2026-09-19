import { invoke } from "@tauri-apps/api/core";
import {
	createContext,
	type ReactNode,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useState,
} from "react";
import { completeClientRestoration } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { ApplicationShutdownBanner } from "./layout/ApplicationShutdownBanner";

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
	const [started, setStarted] = useState(false);
	const ready = status?.phase === "ready";
	const restoring = status?.phase === "restoring";
	const generation = status?.connectionGeneration;
	const [mainShutdownContainer, setMainShutdownContainer] =
		useState<HTMLElement | null>(null);
	const [shutdownContainer, setShutdownContainer] =
		useState<HTMLElement | null>(null);
	useEffect(() => {
		if (ready)
			setMainShutdownContainer(
				document.getElementById("application-shutdown-banner"),
			);
	}, [ready]);
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		let active = true;
		const refresh = async () => {
			try {
				const next = await invoke<DaemonStatus>("get_daemon_status");
				if (!active) return;

				setStatus(next);
				if (next.phase === "ready") setStarted(true);
			} catch (error) {
				if (active) setError(getErrorMessage(error));
			}
		};
		void refresh();
		const timer = setInterval(() => void refresh(), 250);
		return () => {
			active = false;
			clearInterval(timer);
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
						<p role="status">
							{status?.reason ?? "Waiting for the daemon connection."}
						</p>
						{status?.phase === "backoff" && (
							<p>Restart attempt {status.retries} / 3</p>
						)}
						{status?.phase === "stopping" && <div ref={setShutdownContainer} />}
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
			{(started || status?.phase === "stopping") && (
				<ApplicationShutdownBanner
					container={
						status?.phase === "stopping"
							? shutdownContainer
							: mainShutdownContainer
					}
				/>
			)}
		</>
	);
}
