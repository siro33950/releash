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
}

export function EditorPanel({
	tabs,
	activeTab,
	onTabClick,
	onTabClose,
	diffMode,
}: EditorPanelProps) {
	if (tabs.length === 0) {
		return <EmptyState />;
	}

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
					/>
				) : (
					<EmptyState />
				)}
			</div>
		</div>
	);
}
