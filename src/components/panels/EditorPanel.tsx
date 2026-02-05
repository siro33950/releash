import { useGitOriginalContent } from "@/hooks/useGitOriginalContent";
import type { TabInfo } from "@/types/editor";
import { EditorTabs } from "./EditorTabs";
import { EmptyState } from "./EmptyState";
import { type DiffBase, type DiffMode, MonacoDiffViewer } from "./MonacoDiffViewer";

export interface EditorPanelProps {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	onTabClick: (path: string) => void;
	onTabClose: (path: string) => void;
	diffBase: DiffBase;
	diffMode: DiffMode;
	onContentChange?: (path: string, content: string) => void;
}

export function EditorPanel({
	tabs,
	activeTab,
	onTabClick,
	onTabClose,
	diffBase,
	diffMode,
	onContentChange,
}: EditorPanelProps) {
	const originalContent = useGitOriginalContent(
		activeTab?.path ?? null,
		diffBase,
		activeTab?.originalContent ?? "",
	);

	if (tabs.length === 0) {
		return <EmptyState />;
	}

	const handleContentChange = activeTab
		? (content: string) => onContentChange?.(activeTab.path, content)
		: undefined;

	return (
		<div className="flex flex-col h-full">
			<EditorTabs
				tabs={tabs}
				activeTabPath={activeTab?.path ?? null}
				onTabClick={onTabClick}
				onTabClose={onTabClose}
			/>
			<div className="flex-1 overflow-hidden">
				{activeTab ? (
					<MonacoDiffViewer
						key={activeTab.path}
						originalContent={originalContent}
						modifiedContent={activeTab.content}
						language={activeTab.language}
						diffMode={diffMode}
						onContentChange={handleContentChange}
					/>
				) : (
					<EmptyState />
				)}
			</div>
		</div>
	);
}
