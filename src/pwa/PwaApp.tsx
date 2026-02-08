import { FileDiff, GitBranch, MessageSquare, Terminal } from "lucide-react";
import { useCallback, useState } from "react";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import type { LineComment } from "@/types/comment";
import { ConnectionForm } from "./components/ConnectionForm";
import { RemoteCommentList } from "./components/RemoteCommentList";
import { RemoteDiffPanel } from "./components/RemoteDiffPanel";
import { RemoteSourceControl } from "./components/RemoteSourceControl";
import { RemoteTerminalPanel } from "./components/RemoteTerminalPanel";
import { StatusIndicator } from "./components/StatusIndicator";
import { useMessageBus } from "./hooks/useMessageBus";
import { useRemoteFileContent } from "./hooks/useRemoteFileContent";
import { useRemoteGitActions } from "./hooks/useRemoteGitActions";
import { useRemoteGitStatus } from "./hooks/useRemoteGitStatus";
import { useWebSocket } from "./hooks/useWebSocket";

type Tab = "changes" | "diff" | "terminal" | "comments";

const tabs: { id: Tab; label: string; icon: typeof GitBranch }[] = [
	{ id: "changes", label: "Changes", icon: GitBranch },
	{ id: "diff", label: "Diff", icon: FileDiff },
	{ id: "comments", label: "Comments", icon: MessageSquare },
	{ id: "terminal", label: "Terminal", icon: Terminal },
];

export function PwaApp() {
	const [connection, setConnection] = useState<{
		url: string;
		token: string;
	} | null>(null);

	const [selectedPath, setSelectedPath] = useState<string | null>(null);
	const [ptyId, setPtyId] = useState<number | null>(null);
	const [ptyCols, setPtyCols] = useState<number>(80);
	const [activeTab, setActiveTab] = useState<Tab>("changes");
	const [terminalMounted, setTerminalMounted] = useState(false);
	const [comments, setComments] = useState<LineComment[]>([]);

	const { dispatch, subscribe } = useMessageBus();

	const handleMessage = useCallback(
		(msg: import("@/types/protocol").WsMessage) => {
			if (msg.type === "pty_ready") {
				setPtyId(msg.payload.pty_id);
				setPtyCols(msg.payload.cols);
			}
			if (msg.type === "comments_sync") {
				setComments(
					msg.payload.comments.map((c) => ({
						id: c.id,
						filePath: c.file_path,
						lineNumber: c.line_number,
						...(c.end_line != null && { endLine: c.end_line }),
						content: c.content,
						status: c.status,
						createdAt: c.created_at,
					})),
				);
			}
			dispatch(msg);
		},
		[dispatch],
	);

	const { status, send, disconnect } = useWebSocket({
		url: connection?.url ?? "",
		token: connection?.token ?? "",
		onMessage: handleMessage,
	});

	const { stagedFiles, changedFiles } = useRemoteGitStatus({ subscribe });
	const { content, loading, requestContent } = useRemoteFileContent({
		subscribe,
		send,
	});
	const { stage, unstage, error, clearError } = useRemoteGitActions({
		send,
		subscribe,
	});

	const handleConnect = useCallback((wsUrl: string, token: string) => {
		setConnection({ url: wsUrl, token });
	}, []);

	const handleDisconnect = useCallback(() => {
		disconnect();
		setConnection(null);
		setPtyId(null);
	}, [disconnect]);

	const handleSelectFile = useCallback(
		(path: string) => {
			setSelectedPath(path);
			requestContent(path);
		},
		[requestContent],
	);

	const handleNavigateToDiff = useCallback(() => {
		setActiveTab("diff");
	}, []);

	const handleRefreshStatus = useCallback(() => {
		send({ type: "git_status_request", payload: {} as Record<string, never> });
	}, [send]);

	const handleAddComment = useCallback(
		(
			filePath: string,
			lineNumber: number,
			content: string,
			endLine?: number,
		) => {
			send({
				type: "add_comment",
				payload: {
					file_path: filePath,
					line_number: lineNumber,
					...(endLine != null && { end_line: endLine }),
					content,
				},
			});
			const comment: LineComment = {
				id: `pwa-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
				filePath,
				lineNumber,
				...(endLine != null && { endLine }),
				content,
				status: "unsent",
				createdAt: Date.now(),
			};
			setComments((prev) => [...prev, comment]);
		},
		[send],
	);

	const handleSendToTerminal = useCallback(
		(unsent: LineComment[]) => {
			const text = formatCommentsForTerminal(unsent);
			if (!text) return;
			if (ptyId != null) {
				send({
					type: "pty_input",
					payload: { pty_id: ptyId, data: `${text}\n` },
				});
				setComments((prev) =>
					prev.map((c) =>
						unsent.some((u) => u.id === c.id)
							? { ...c, status: "sent" as const }
							: c,
					),
				);
			}
		},
		[send, ptyId],
	);

	if (!connection) {
		return <ConnectionForm onConnect={handleConnect} />;
	}

	return (
		<div className="flex flex-col h-dvh bg-neutral-950 text-neutral-100">
			<header className="flex items-center justify-between px-3 py-1.5 border-b border-neutral-800 bg-neutral-900 shrink-0">
				<h1 className="text-sm font-semibold">Releash Remote</h1>
				<div className="flex items-center gap-2">
					<StatusIndicator status={status} />
					<button
						type="button"
						onClick={handleDisconnect}
						className="text-xs px-2 py-0.5 rounded bg-neutral-800 hover:bg-neutral-700 transition-colors"
					>
						切断
					</button>
				</div>
			</header>

			<main className="flex-1 overflow-hidden relative">
				<div
					className="absolute inset-0"
					style={{ display: activeTab === "changes" ? undefined : "none" }}
				>
					<RemoteSourceControl
						stagedFiles={stagedFiles}
						changedFiles={changedFiles}
						selectedPath={selectedPath}
						onSelectFile={handleSelectFile}
						onStage={stage}
						onUnstage={unstage}
						error={error}
						onClearError={clearError}
						onNavigateToDiff={handleNavigateToDiff}
						onRefresh={handleRefreshStatus}
					/>
				</div>

				<div
					className="absolute inset-0 flex flex-col"
					style={{ display: activeTab === "diff" ? undefined : "none" }}
				>
					{selectedPath && (
						<div className="flex items-center px-3 py-1 border-b border-neutral-800 bg-neutral-900 shrink-0">
							<span className="text-xs text-neutral-500 truncate">
								{selectedPath}
							</span>
						</div>
					)}
					<div className="flex-1" style={{ minHeight: 0 }}>
						{status === "connected" ? (
							<RemoteDiffPanel
								path={selectedPath}
								original={content?.original ?? ""}
								modified={content?.modified ?? ""}
								loading={loading}
								onAddComment={handleAddComment}
							/>
						) : (
							<div className="flex items-center justify-center h-full text-neutral-500">
								<p>接続中...</p>
							</div>
						)}
					</div>
				</div>

				<div
					className="absolute inset-0"
					style={{ display: activeTab === "comments" ? undefined : "none" }}
				>
					<RemoteCommentList
						comments={comments}
						onSendToTerminal={handleSendToTerminal}
					/>
				</div>

				<div
					className="absolute inset-0"
					style={{
						visibility: activeTab === "terminal" ? "visible" : "hidden",
						pointerEvents: activeTab === "terminal" ? "auto" : "none",
					}}
				>
					{terminalMounted && status === "connected" && ptyId != null ? (
						<RemoteTerminalPanel
							ptyId={ptyId}
							ptyCols={ptyCols}
							send={send}
							subscribe={subscribe}
							visible={activeTab === "terminal"}
						/>
					) : activeTab === "terminal" &&
						status === "connected" &&
						ptyId == null ? (
						<div className="flex items-center justify-center h-full text-neutral-500">
							<p>デスクトップのターミナルがまだ起動していません</p>
						</div>
					) : null}
				</div>
			</main>

			<nav className="flex shrink-0 border-t border-neutral-800 bg-neutral-900">
				{tabs.map((tab) => {
					const Icon = tab.icon;
					const isActive = activeTab === tab.id;
					return (
						<button
							key={tab.id}
							type="button"
							className={`flex-1 flex flex-col items-center justify-center h-12 gap-0.5 transition-colors ${
								isActive
									? "text-blue-400 border-t-2 border-blue-400"
									: "text-neutral-500"
							}`}
							onClick={() => {
								setActiveTab(tab.id);
								if (tab.id === "terminal") setTerminalMounted(true);
							}}
						>
							<Icon className="h-4 w-4" />
							<span className="text-[10px]">{tab.label}</span>
						</button>
					);
				})}
			</nav>
		</div>
	);
}
