import type { DiffBase } from "../hooks/useRemoteFileContent";
import type { ConnectionStatus } from "../hooks/useWebSocket";
import { RemoteDiffPanel } from "./RemoteDiffPanel";

interface DiffTabContentProps {
	status: ConnectionStatus;
	selectedPath: string | null;
	diffBase: DiffBase;
	hasDiffChanges: boolean;
	stagedFiles: { path: string }[];
	content: { original: string; modified: string; staged: string | null } | null;
	loading: boolean;
	onDiffBaseChange: (base: DiffBase) => void;
	onStageAll: () => void;
	onUnstageAll: () => void;
	onStageHunk: (patch: string) => void;
	onAddComment: (
		filePath: string,
		lineNumber: number,
		content: string,
		endLine?: number,
	) => void;
}

export function DiffTabContent({
	status,
	selectedPath,
	diffBase,
	hasDiffChanges,
	stagedFiles,
	content,
	loading,
	onDiffBaseChange,
	onStageAll,
	onUnstageAll,
	onStageHunk,
	onAddComment,
}: DiffTabContentProps) {
	return (
		<>
			{selectedPath && (
				<div className="flex items-center justify-between gap-2 px-3 py-1 border-b border-border bg-card shrink-0">
					<span className="text-xs text-muted-foreground truncate flex-1 min-w-0">
						{selectedPath}
					</span>
					<div className="flex items-center gap-1.5 shrink-0">
						<select
							value={diffBase}
							onChange={(e) => onDiffBaseChange(e.target.value as DiffBase)}
							className="text-xs bg-input text-secondary-foreground border border-border rounded px-1.5 py-0.5"
						>
							<option value="HEAD">HEAD</option>
							<option value="staged">Staged</option>
						</select>
						{hasDiffChanges && (
							<button
								type="button"
								onClick={onStageAll}
								className="text-xs px-2 py-0.5 rounded bg-success/80 hover:bg-success/70 text-success-foreground transition-colors"
							>
								Stage All
							</button>
						)}
						{diffBase === "HEAD" &&
							stagedFiles.some((f) => f.path === selectedPath) && (
								<button
									type="button"
									onClick={onUnstageAll}
									className="text-xs px-2 py-0.5 rounded bg-warning/80 hover:bg-warning/70 text-warning-foreground transition-colors"
								>
									Unstage All
								</button>
							)}
					</div>
				</div>
			)}
			<div className="flex-1" style={{ minHeight: 0 }}>
				{status === "connected" ? (
					<RemoteDiffPanel
						key={selectedPath}
						path={selectedPath}
						original={content?.original ?? ""}
						modified={content?.modified ?? ""}
						loading={loading}
						diffBase={diffBase}
						staged={content?.staged ?? null}
						onStageHunk={onStageHunk}
						onAddComment={onAddComment}
					/>
				) : (
					<div className="flex items-center justify-center h-full text-muted-foreground">
						<p>接続中...</p>
					</div>
				)}
			</div>
		</>
	);
}
