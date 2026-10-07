import { invoke } from "@tauri-apps/api/core";
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
import {
	getClient,
	getConnectionState,
	onConnectionStateChange,
	reconnectClient,
} from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

const DaemonReadyContext = createContext(true);
export const useDaemonReady = () => useContext(DaemonReadyContext);
type Failure = { message: string; serverOlder: boolean };

export function DaemonBoundary({ children }: { children: ReactNode }) {
	const [connection, setConnection] = useState(getConnectionState);
	const [shown, setShown] = useState(() => getConnectionState() === "READY");
	const [failure, setFailure] = useState<Failure | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [clientError, setClientError] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const ready = connection === "READY";
	useEffect(() => {
		let active = true;
		const update = () => {
			const next = getConnectionState();
			setConnection(next);
			if (next === "READY") {
				setShown(true);
				setFailure(null);
			}
			if (next === "TRANSIENT_FAILURE") {
				void invoke<Failure | null>("get_desktop_connection_failure")
					.then((value) => {
						if (active) setFailure(value);
					})
					.catch((error) => {
						if (active) setError(getErrorMessage(error));
					});
			}
		};
		const release = onConnectionStateChange(update);
		update();
		void getClient().catch((error) => {
			if (active) setError(getErrorMessage(error));
		});
		const onError = (event: Event) =>
			setClientError((event as CustomEvent<string>).detail);
		window.addEventListener("releash-client-error", onError);
		return () => {
			active = false;
			release();
			window.removeEventListener("releash-client-error", onError);
		};
	}, []);
	const action = async (command: string) => {
		setBusy(true);
		try {
			await invoke(command);
			setError(null);
			if (command !== "quit_desktop") reconnectClient();
		} catch (error) {
			setError(getErrorMessage(error));
		} finally {
			setBusy(false);
		}
	};
	return (
		<>
			{clientError && (
				<div
					role="alert"
					className="fixed bottom-4 right-4 z-[110] rounded border bg-background p-3"
				>
					{clientError}
					<button
						type="button"
						aria-label="Dismiss error"
						onClick={() => setClientError(null)}
					>
						×
					</button>
				</div>
			)}
			{shown && (
				<DaemonReadyContext.Provider value={ready}>
					<div className="contents" inert={!ready} aria-hidden={!ready}>
						{children}
					</div>
				</DaemonReadyContext.Provider>
			)}
			{!ready && (
				<Dialog open>
					<DialogContent
						showCloseButton={false}
						onEscapeKeyDown={(event) => event.preventDefault()}
						onInteractOutside={(event) => event.preventDefault()}
					>
						<DialogTitle>
							{failure
								? "サーバに接続できません"
								: connection === "TRANSIENT_FAILURE"
									? "サーバは動いていません"
									: "サーバへ接続中"}
						</DialogTitle>
						<DialogDescription
							role="status"
							data-client-connection={connection}
						>
							{failure?.message ?? "サーバへの接続を待っています。"}
						</DialogDescription>
						{error && <p role="alert">{error}</p>}
						{connection === "TRANSIENT_FAILURE" && (
							<button
								type="button"
								disabled={busy}
								onClick={() =>
									void action(
										failure?.serverOlder ? "replace_daemon" : "start_daemon",
									)
								}
							>
								{failure?.serverOlder
									? "サーバを停止して起動し直す"
									: "サーバを起動"}
							</button>
						)}
						<button
							type="button"
							disabled={busy}
							onClick={() => void action("quit_desktop")}
						>
							Quit
						</button>
					</DialogContent>
				</Dialog>
			)}
		</>
	);
}
