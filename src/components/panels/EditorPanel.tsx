import type { TabInfo } from "@/types/editor";
import { EditorTabs } from "./EditorTabs";
import { EmptyState } from "./EmptyState";
import { type DiffMode, MonacoDiffViewer } from "./MonacoDiffViewer";

export interface EditorPanelProps {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	onTabClick: (path: string) => void;
	onTabClose: (path: string) => void;
	diffMode: DiffMode;
	onContentChange?: (path: string, content: string) => void;
}

export function EditorPanel({
	tabs,
	activeTab,
	onTabClick,
	onTabClose,
	diffMode,
	onContentChange,
}: EditorPanelProps) {
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
						originalContent={activeTab.originalContent}
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
