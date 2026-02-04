import { open } from "@tauri-apps/plugin-dialog";
import { ChevronsDownUp, FolderOpen, RefreshCw } from "lucide-react";
import { useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useFileTree } from "@/hooks/useFileTree";
import { FileTree } from "./FileTree";

export interface SidebarPanelProps {
	onSelectFile?: (path: string) => void;
}

export function SidebarPanel({ onSelectFile }: SidebarPanelProps) {
	const [rootPath, setRootPath] = useState<string | null>(null);
	const [selectedPath, setSelectedPath] = useState<string | null>(null);

	const {
		tree,
		expandedPaths,
		loading,
		error,
		toggleExpand,
		refresh,
		collapseAll,
	} = useFileTree({
		rootPath,
	});

	const handleOpenFolder = async () => {
		const selected = await open({ directory: true });
		if (selected) {
			setRootPath(selected);
			setSelectedPath(null);
		}
	};

	const handleSelectFile = (path: string) => {
		setSelectedPath(path);
		onSelectFile?.(path);
	};

	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center justify-end h-[30px] px-3 border-b border-border">
				<div className="flex items-center gap-1">
					<button
						type="button"
						onClick={refresh}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="Refresh"
						aria-label="Refresh"
						disabled={!rootPath}
					>
						<RefreshCw className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
					<button
						type="button"
						onClick={collapseAll}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="Collapse All"
						aria-label="Collapse All"
						disabled={!rootPath}
					>
						<ChevronsDownUp className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
					<button
						type="button"
						onClick={handleOpenFolder}
						className="p-1 hover:bg-sidebar-accent rounded transition-colors"
						title="Open Folder"
						aria-label="Open Folder"
					>
						<FolderOpen className="h-3.5 w-3.5 text-muted-foreground" />
					</button>
				</div>
			</div>
			<ScrollArea className="flex-1">
				<div className="p-2">
					{loading && (
						<div className="px-2 py-4 text-sm text-muted-foreground">
							Loading...
						</div>
					)}

					{error && (
						<div className="px-2 py-4 text-sm text-destructive">{error}</div>
					)}

					{!rootPath && !loading && (
						<div className="px-2 py-4 text-center">
							<button
								type="button"
								onClick={handleOpenFolder}
								className="inline-flex items-center gap-2 px-4 py-2 text-sm bg-sidebar-accent hover:bg-sidebar-accent/80 rounded transition-colors"
							>
								<FolderOpen className="h-4 w-4" />
								Open Folder
							</button>
						</div>
					)}

					{rootPath && !loading && !error && (
						<FileTree
							tree={tree}
							selectedPath={selectedPath}
							expandedPaths={expandedPaths}
							onSelect={handleSelectFile}
							onToggleExpand={toggleExpand}
						/>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
